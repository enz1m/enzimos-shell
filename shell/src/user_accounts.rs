use enzui_shell::i18n::tr;
mod password;
pub use enzui_core::form::SecretText;
use std::collections::HashMap;
use zbus::{
    Connection, Proxy,
    zvariant::{OwnedObjectPath, Value},
};
const SERVICE: &str = "org.freedesktop.Accounts";
const ROOT: &str = "/org/freedesktop/Accounts";
const USER: &str = "org.freedesktop.Accounts.User";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub uid: u32,
    pub name: String,
    pub display: String,
    pub admin: bool,
    pub locked: bool,
    pub password_mode: i32,
    pub icon: Option<String>,
    pub local: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Accounts {
    pub users: Vec<Account>,
    pub sessions: Vec<u32>,
    pub current: u32,
    pub administrator: bool,
}
#[derive(Clone, Debug)]
pub enum Action {
    Create {
        name: String,
        display: String,
        admin: bool,
        password: SecretText,
    },
    Rename(u32, String),
    Avatar(u32, String),
    Administrator(u32, bool),
    Locked(u32, bool),
    Password(u32, SecretText),
    Remove(u32),
}

fn clean(s: String) -> String {
    s.chars().filter(|c| !c.is_control()).take(160).collect()
}
pub const MAX_PASSWORD_BYTES: usize = 511;
pub fn valid_password(value: &str) -> bool {
    value.len() <= MAX_PASSWORD_BYTES
        && value.chars().count() >= 8
        && !value.chars().all(char::is_whitespace)
        && !value.chars().any(char::is_control)
}
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name.as_bytes()[0].is_ascii_lowercase()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        && !["root", "nobody", "greetd", "greeter"].contains(&name)
}
fn valid_display(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 160 && !name.chars().any(char::is_control)
}
fn failure(e: zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(name, _, _) if name.as_str().contains("UserExists") => {
            tr("shell-this-username-is-already-in-use").into()
        }
        zbus::Error::MethodError(name, _, _)
            if name.as_str().contains("Permission")
                || name.as_str().contains("Denied")
                || name.as_str().contains("Cancelled") =>
        {
            tr("shell-authorization-was-cancelled-or-denied-no-change-was-approved").into()
        }
        _ => {
            tr("shell-the-account-service-could-not-complete-this-change-refresh-the-account-list")
                .into()
        }
    }
}
// Reuse the polkit subject connection across Unlock and subsequent account calls.
async fn connection() -> Result<Connection, String> {
    static BUS: tokio::sync::Mutex<Option<Connection>> = tokio::sync::Mutex::const_new(None);
    let mut bus = BUS.lock().await;
    if bus.as_ref().is_none_or(Connection::is_closed) {
        *bus = Some(
            Connection::system()
                .await
                .map_err(|_| tr("shell-no-system-account-service"))?,
        );
    }
    Ok(bus.as_ref().expect("account bus").clone())
}
async fn manager(c: &Connection) -> Result<Proxy<'_>, String> {
    Proxy::new(c, SERVICE, ROOT, SERVICE).await.map_err(failure)
}
async fn user(c: &Connection, uid: u32) -> Result<Proxy<'_>, String> {
    let path: OwnedObjectPath = manager(c)
        .await?
        .call("FindUserById", &(i64::from(uid),))
        .await
        .map_err(failure)?;
    Proxy::new(c, SERVICE, path, USER).await.map_err(failure)
}
pub(crate) async fn current_profile() -> Option<(String, String, Option<String>)> {
    let connection = connection().await.ok()?;
    let profile = user(&connection, rustix::process::getuid().as_raw())
        .await
        .ok()?;
    let user = clean(profile.get_property("UserName").await.ok()?);
    let name = clean(profile.get_property("RealName").await.unwrap_or_default());
    let name = if name.trim().is_empty() {
        user.clone()
    } else {
        name
    };
    let icon: String = profile.get_property("IconFile").await.unwrap_or_default();
    Some((user, name, (!icon.is_empty()).then_some(icon)))
}
async fn read(c: &Connection, current: u32) -> Result<Accounts, String> {
    let manager = manager(c).await?;
    let paths: Vec<OwnedObjectPath> = manager
        .call("ListCachedUsers", &())
        .await
        .map_err(|_| tr("shell-user-management-is-unavailable-on-this-installation"))?;
    if paths.len() > 256 {
        return Err(tr("shell-account-service-returned-too-many-accounts").into());
    }
    let mut users = Vec::new();
    for path in paths {
        let p = Proxy::new(c, SERVICE, path, USER).await.map_err(failure)?;
        let uid: u64 = p.get_property("Uid").await.map_err(failure)?;
        let uid = u32::try_from(uid).map_err(|_| tr("shell-invalid-account-identifier"))?;
        if p.get_property::<bool>("SystemAccount")
            .await
            .unwrap_or(true)
            && uid != current
        {
            continue;
        }
        let name = clean(p.get_property("UserName").await.map_err(failure)?);
        let display = clean(
            p.get_property("RealName")
                .await
                .unwrap_or_else(|_| name.clone()),
        );
        let display = if display.trim().is_empty() {
            name.clone()
        } else {
            display
        };
        let icon: String = p.get_property("IconFile").await.unwrap_or_default();
        users.push(Account {
            uid,
            name,
            display,
            admin: p.get_property::<i32>("AccountType").await.unwrap_or(0) == 1,
            locked: p.get_property("Locked").await.unwrap_or(true),
            password_mode: p.get_property("PasswordMode").await.unwrap_or(0),
            icon: (!icon.is_empty()).then_some(icon),
            local: p.get_property("LocalAccount").await.unwrap_or(false),
        });
    }
    users.sort_by(|a, b| {
        (a.uid != current, a.display.to_lowercase())
            .cmp(&(b.uid != current, b.display.to_lowercase()))
    });
    let administrator = current == 0 || users.iter().any(|u| u.uid == current && u.admin);
    let login = Proxy::new(
        c,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await
    .map_err(failure)?;
    let sessions: Vec<(String, u32, String, String, OwnedObjectPath)> = login
        .call("ListSessions", &())
        .await
        .map_err(|_| tr("shell-cannot-check-active-sessions-account-removal-is-unavailable"))?;
    Ok(Accounts {
        users,
        sessions: sessions.into_iter().map(|s| s.1).collect(),
        current,
        administrator,
    })
}
impl Accounts {
    pub fn removable(&self, uid: u32) -> bool {
        self.users.iter().any(|u| u.uid == uid && u.local)
            && self.current != uid
            && !self.sessions.contains(&uid)
            && self.may_disable(uid)
    }
    pub fn may_disable(&self, uid: u32) -> bool {
        self.users.iter().find(|u| u.uid == uid).is_some_and(|u| {
            !u.admin
                || self
                    .users
                    .iter()
                    .any(|other| other.uid != uid && other.admin && !other.locked)
        })
    }
    pub fn validate_action(&self, action: &Action) -> Result<(), String> {
        if let Action::Create {
            name,
            display,
            password,
            ..
        } = action
        {
            if !self.administrator {
                return Err(tr("shell-an-administrator-must-add-accounts").into());
            }
            if !valid_name(name) || !valid_display(display) {
                return Err(tr(
                    "shell-enter-a-name-and-a-lowercase-username-up-to-32-letters-numbers-or",
                )
                .into());
            }
            if !valid_password(password.expose_secret()) {
                return Err(tr("shell-use-a-password-with-at-least-8-characters-and-no-control-characters-up-to-51").into());
            }
            if self.users.iter().any(|u| u.name == *name) {
                return Err(tr("shell-this-username-is-already-in-use").into());
            }
            return Ok(());
        }
        let uid = match action {
            Action::Rename(uid, _)
            | Action::Avatar(uid, _)
            | Action::Administrator(uid, _)
            | Action::Locked(uid, _)
            | Action::Password(uid, _)
            | Action::Remove(uid) => *uid,
            _ => unreachable!(),
        };
        let account = self
            .users
            .iter()
            .find(|u| u.uid == uid)
            .ok_or_else(|| tr("shell-this-account-no-longer-exists"))?;
        if !account.local {
            return Err(tr("shell-this-account-is-managed-by-your-organization-2").into());
        }
        let own = uid == self.current;
        if !self.administrator
            && !(own && matches!(action, Action::Rename(..) | Action::Avatar(..)))
        {
            return Err(tr("shell-an-administrator-must-approve-this-change").into());
        }
        match action {
            Action::Locked(..) if account.password_mode != 0 => {
                Err(tr("shell-set-a-regular-password-before-changing-sign-in-availability").into())
            }
            Action::Rename(_, name) if !valid_display(name) => {
                Err(tr("shell-enter-a-name-without-control-characters-up-to-160-bytes").into())
            }
            Action::Avatar(_, path)
                if !path.is_empty()
                    && (!path.starts_with('/')
                        || path.len() > 4096
                        || path.chars().any(char::is_control)) =>
            {
                Err(tr("shell-choose-a-local-profile-image").into())
            }
            Action::Administrator(_, false) | Action::Locked(_, true) if !self.may_disable(uid) => {
                Err(tr("shell-keep-at-least-one-unlocked-administrator").into())
            }
            Action::Locked(_, true) if own => {
                Err(tr("shell-you-cannot-disable-your-own-active-account").into())
            }
            Action::Password(_, value) if !valid_password(value.expose_secret()) => Err(tr("shell-use-a-password-with-at-least-8-characters-and-no-control-characters-up-to-51").into()),
            Action::Remove(_) if !self.removable(uid) => Err(
                tr("shell-sign-this-user-out-first-keep-your-own-account-and-an-unlocked-administrator")
                    .into(),
            ),
            _ => Ok(()),
        }
    }
}
pub async fn load() -> Result<Accounts, String> {
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        let c = connection().await?;
        read(&c, rustix::process::getuid().as_raw()).await
    })
    .await
    .map_err(|_| tr("shell-account-service-did-not-respond").to_owned())?
}
async fn unlock_inner() -> Result<(), String> {
    let c = connection().await?;
    let own = c
        .unique_name()
        .ok_or_else(|| tr("shell-no-account-service-identity"))?
        .as_str();
    let mut subject = HashMap::new();
    subject.insert("name", Value::from(own));
    let p = Proxy::new(
        &c,
        "org.freedesktop.PolicyKit1",
        "/org/freedesktop/PolicyKit1/Authority",
        "org.freedesktop.PolicyKit1.Authority",
    )
    .await
    .map_err(failure)?;
    let (authorized, _, _): (bool, bool, HashMap<String, String>) = p
        .call_with_flags(
            "CheckAuthorization",
            zbus::proxy::MethodFlags::AllowInteractiveAuth.into(),
            &(
                ("system-bus-name", subject),
                "org.freedesktop.accounts.user-administration",
                HashMap::<String, String>::new(),
                1_u32,
                "",
            ),
        )
        .await
        .map_err(failure)?
        .ok_or_else(|| tr("shell-authorization-did-not-finish"))?;
    if authorized {
        Ok(())
    } else {
        Err(tr("shell-administrator-authorization-was-cancelled-or-denied").into())
    }
}
async fn apply(c: &Connection, action: Action, current: u32) -> Result<(), String> {
    let accounts = read(c, current).await?;
    accounts.validate_action(&action)?;
    let flags = zbus::proxy::MethodFlags::AllowInteractiveAuth.into();
    match action {
        Action::Create {
            name,
            display,
            admin,
            password,
        } => {
            let encoded = password::hash(password).await?;
            let path: OwnedObjectPath = manager(c)
                .await?
                .call_with_flags("CreateUser", flags, &(name, display, i32::from(admin)))
                .await
                .map_err(failure)?
                .ok_or_else(|| tr("shell-account-creation-did-not-finish"))?;
            let p = Proxy::new(c, SERVICE, path, USER).await.map_err(failure)?;
            p.call_with_flags::<_,_,()>("SetPassword",flags,&(encoded.expose_secret(), "")).await.map_err(|_| tr("shell-account-was-created-but-password-setup-did-not-finish-refresh-accounts-and-c"))?;
        }
        Action::Remove(uid) => {
            manager(c)
                .await?
                .call_with_flags::<_, _, ()>("DeleteUser", flags, &(i64::from(uid), false))
                .await
                .map_err(failure)?;
        }
        action => {
            let uid = match &action {
                Action::Rename(uid, _)
                | Action::Avatar(uid, _)
                | Action::Administrator(uid, _)
                | Action::Locked(uid, _)
                | Action::Password(uid, _) => *uid,
                _ => unreachable!(),
            };
            let p = user(c, uid).await?;
            match action {
                Action::Rename(_, name) => {
                    p.call_with_flags::<_, _, ()>("SetRealName", flags, &(name.trim(),))
                        .await
                        .map_err(failure)?;
                }
                Action::Avatar(_, path) => {
                    p.call_with_flags::<_, _, ()>("SetIconFile", flags, &(path,))
                        .await
                        .map_err(failure)?;
                }
                Action::Administrator(_, admin) => {
                    p.call_with_flags::<_, _, ()>("SetAccountType", flags, &(i32::from(admin),))
                        .await
                        .map_err(failure)?;
                }
                Action::Password(_, password) => {
                    let encoded = password::hash(password).await?;
                    p.call_with_flags::<_, _, ()>(
                        "SetPassword",
                        flags,
                        &(encoded.expose_secret(), ""),
                    )
                    .await
                    .map_err(failure)?;
                }
                Action::Locked(_, locked) => {
                    p.call_with_flags::<_, _, ()>("SetLocked", flags, &(locked,))
                        .await
                        .map_err(failure)?;
                }
                _ => unreachable!(),
            }
        }
    }
    Ok(())
}
pub async fn change(action: Action) -> Result<Accounts, String> {
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let c = connection().await?;
        let uid = rustix::process::getuid().as_raw();
        apply(&c, action, uid).await?;
        read(&c, uid).await
    })
    .await
    .map_err(|_| {
        tr("shell-authorization-timed-out-refresh-accounts-before-trying-again").to_owned()
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn account(uid: u32, admin: bool) -> Account {
        Account {
            uid,
            name: format!("user{uid}"),
            display: "Person".into(),
            admin,
            locked: false,
            password_mode: 0,
            icon: None,
            local: true,
        }
    }
    #[test]
    fn user_accounts_behaviour() {
        // Password validation and action formatting do not expose credentials.
        {
            assert!(valid_password("test-only-initial-password"));
            for value in ["short", "        ", "password\n", "password\0"] {
                assert!(!valid_password(value));
            }
            assert!(!valid_password(&"x".repeat(MAX_PASSWORD_BYTES + 1)));
            let action =
                Action::Password(1001, SecretText::new("test-only-private-password".into()));
            assert!(!format!("{action:?}").contains("private"));
        }
        // Pending first login cannot be put in an unrecoverable empty password lock.
        {
            let mut pending = account(1001, false);
            pending.password_mode = 1;
            let data = Accounts {
                users: vec![account(1000, true), pending],
                current: 1000,
                administrator: true,
                sessions: vec![1000],
            };
            assert!(data.validate_action(&Action::Locked(1001, true)).is_err());
            assert!(data.validate_action(&Action::Remove(1001)).is_ok());
        }
        // Protects current session and last administrator.
        {
            let mut a = Accounts {
                users: vec![account(1000, true), account(1001, false)],
                current: 1000,
                sessions: vec![1001],
                administrator: true,
            };
            assert!(!a.removable(1000));
            assert!(!a.removable(1001));
            assert!(
                a.validate_action(&Action::Administrator(1000, false))
                    .is_err()
            );
            assert!(a.validate_action(&Action::Locked(1000, true)).is_err());
            a.sessions.clear();
            assert!(a.removable(1001));
            a.users[1].admin = true;
            assert!(
                a.validate_action(&Action::Administrator(1000, false))
                    .is_ok()
            );
            a.administrator = false;
            assert!(
                a.validate_action(&Action::Rename(1000, "Alex".into()))
                    .is_ok()
            );
            assert!(
                a.validate_action(&Action::Rename(1001, "Alex".into()))
                    .is_err()
            );
        }
        // Refuses reserved and malformed names.
        {
            for name in [
                "",
                "Root",
                "root",
                "greeter",
                "a\nb",
                "-alice",
                "a:b",
                "alice smith",
            ] {
                assert!(!valid_name(name));
            }
            assert!(valid_name("alex-2"));
        }
    }
}

#[cfg(test)]
mod service_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    #[derive(Default)]
    struct State {
        created: bool,
        calls: Vec<String>,
        name: String,
    }
    struct Manager(Arc<Mutex<State>>);
    #[zbus::interface(name = "org.freedesktop.Accounts")]
    impl Manager {
        fn list_cached_users(&self) -> Vec<OwnedObjectPath> {
            let mut paths = vec!["/org/freedesktop/Accounts/User1000"];
            if self.0.lock().unwrap().created {
                paths.push("/org/freedesktop/Accounts/User1001");
            }
            paths
                .into_iter()
                .map(|p| OwnedObjectPath::try_from(p).unwrap())
                .collect()
        }
        fn find_user_by_id(&self, uid: i64) -> zbus::fdo::Result<OwnedObjectPath> {
            if ![1000, 1001].contains(&uid) {
                return Err(zbus::fdo::Error::Failed("unknown user".into()));
            }
            OwnedObjectPath::try_from(format!("/org/freedesktop/Accounts/User{uid}"))
                .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
        }
        fn create_user(&self, name: &str, display: &str, kind: i32) -> OwnedObjectPath {
            let mut s = self.0.lock().unwrap();
            s.created = true;
            s.calls.push(format!("create:{name}:{display}:{kind}"));
            OwnedObjectPath::try_from("/org/freedesktop/Accounts/User1001").unwrap()
        }
        fn delete_user(&self, uid: i64, remove_files: bool) {
            let mut s = self.0.lock().unwrap();
            s.created = false;
            s.calls.push(format!("delete:{uid}:{remove_files}"));
        }
    }
    struct User(Arc<Mutex<State>>, u32);
    #[zbus::interface(name = "org.freedesktop.Accounts.User")]
    impl User {
        #[zbus(property)]
        fn uid(&self) -> u64 {
            u64::from(self.1)
        }
        #[zbus(property)]
        fn user_name(&self) -> String {
            if self.1 == 1000 { "alex" } else { "sam" }.into()
        }
        #[zbus(property)]
        fn real_name(&self) -> String {
            let s = self.0.lock().unwrap();
            if s.name.is_empty() {
                "Alex Morgan".into()
            } else {
                s.name.clone()
            }
        }
        #[zbus(property)]
        fn account_type(&self) -> i32 {
            i32::from(self.1 == 1000)
        }
        #[zbus(property)]
        fn system_account(&self) -> bool {
            false
        }
        #[zbus(property)]
        fn local_account(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn locked(&self) -> bool {
            false
        }
        #[zbus(property)]
        fn password_mode(&self) -> i32 {
            0
        }
        #[zbus(property)]
        fn icon_file(&self) -> String {
            String::new()
        }
        fn set_real_name(&self, name: &str) {
            self.0.lock().unwrap().name = name.into();
        }
        fn set_icon_file(&self, path: &str) {
            self.0.lock().unwrap().calls.push(format!("icon:{path}"));
        }
        fn set_password(&self, encoded: &str, hint: &str) {
            assert!(encoded.starts_with("$y$") || encoded.starts_with("$6$"));
            assert!(hint.is_empty());
            self.0.lock().unwrap().calls.push("password-set".into());
        }
    }
    struct Login;
    #[zbus::interface(name = "org.freedesktop.login1.Manager")]
    impl Login {
        fn list_sessions(&self) -> Vec<(String, u32, String, String, OwnedObjectPath)> {
            vec![(
                "test".into(),
                1000,
                "alex".into(),
                "seat0".into(),
                OwnedObjectPath::try_from("/org/freedesktop/login1/session/test").unwrap(),
            )]
        }
    }
    struct Bus(std::process::Child);
    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    #[tokio::test]
    async fn actual_dbus_contract_creates_password_account_and_keeps_home_on_removal() {
        use std::io::BufRead;
        let mut daemon = Bus(std::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("private bus"));
        let mut address = String::new();
        std::io::BufReader::new(daemon.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let address = address.trim();
        let state = Arc::new(Mutex::new(State::default()));
        let service = zbus::connection::Builder::address(address)
            .unwrap()
            .name(SERVICE)
            .unwrap()
            .name("org.freedesktop.login1")
            .unwrap()
            .serve_at(ROOT, Manager(std::sync::Arc::clone(&state)))
            .unwrap()
            .serve_at(
                "/org/freedesktop/Accounts/User1000",
                User(std::sync::Arc::clone(&state), 1000),
            )
            .unwrap()
            .serve_at(
                "/org/freedesktop/Accounts/User1001",
                User(std::sync::Arc::clone(&state), 1001),
            )
            .unwrap()
            .serve_at("/org/freedesktop/login1", Login)
            .unwrap()
            .build()
            .await
            .unwrap();
        let client = zbus::connection::Builder::address(address)
            .unwrap()
            .build()
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            let list = read(&client, 1000).await.unwrap();
            assert!(list.administrator);
            assert_eq!(list.users.len(), 1);
            apply(&client, Action::Rename(1000, "Alex Updated".into()), 1000)
                .await
                .unwrap();
            assert_eq!(
                read(&client, 1000).await.unwrap().users[0].display,
                "Alex Updated"
            );
            apply(
                &client,
                Action::Create {
                    name: "sam".into(),
                    display: "Sam Rivera".into(),
                    admin: false,
                    password: SecretText::new("test-only-initial-password".into()),
                },
                1000,
            )
            .await
            .unwrap();
            let created = read(&client, 1000).await.unwrap();
            assert_eq!(created.users.len(), 2);
            assert_eq!(
                created
                    .users
                    .iter()
                    .find(|u| u.uid == 1001)
                    .unwrap()
                    .password_mode,
                0
            );
            apply(
                &client,
                Action::Avatar(1000, "/fixture/avatar.png".into()),
                1000,
            )
            .await
            .unwrap();
            assert!(apply(&client, Action::Remove(1000), 1000).await.is_err());
            assert!(
                apply(&client, Action::Administrator(1000, false), 1000)
                    .await
                    .is_err()
            );
            apply(&client, Action::Remove(1001), 1000).await.unwrap();
            let calls = state.lock().unwrap().calls.clone();
            assert_eq!(
                calls,
                vec![
                    "create:sam:Sam Rivera:0",
                    "password-set",
                    "icon:/fixture/avatar.png",
                    "delete:1001:false"
                ]
            );
        })
        .await
        .expect("private account service responded");
        drop(client);
        drop(service);
        drop(daemon);
    }
}

pub async fn unlock() -> Result<(), String> {
    tokio::time::timeout(std::time::Duration::from_secs(120), unlock_inner())
        .await
        .map_err(|_| tr("shell-administrator-authorization-timed-out").to_owned())?
}
