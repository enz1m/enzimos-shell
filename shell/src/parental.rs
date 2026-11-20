//! AccountsService malcontent app filtering for cooperating launchers.
//! Administrator accounts bypass the filter; arbitrary terminal commands remain outside its
//! scope.

use enzui_shell::i18n::tr;
use std::path::{Path, PathBuf};

const ACCOUNTS: &str = "org.freedesktop.Accounts";
const FILTER: &str = "com.endlessm.ParentalControls.AppFilter";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppFilter {
    /// `true`: only the listed apps may start; `false`: the listed may not.
    pub allowlist: bool,
    /// Absolute program paths and Flatpak refs (`app/ID/ARCH/BRANCH`).
    pub entries: Vec<String>,
}

impl AppFilter {
    /// Whether an app with this Flatpak ID or program path may start.
    #[must_use]
    pub fn allows(&self, flatpak: Option<&str>, program: Option<&Path>) -> bool {
        let listed = self
            .entries
            .iter()
            .any(|entry| match entry.strip_prefix("app/") {
                Some(reference) => {
                    flatpak.is_some_and(|id| reference.split('/').next() == Some(id))
                }
                None => program.is_some_and(|path| Path::new(entry) == path),
            });
        if self.allowlist { listed } else { !listed }
    }
    #[must_use]
    pub fn restricts(&self) -> bool {
        self.allowlist || !self.entries.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub uid: u32,
    pub name: String,
    pub real_name: String,
    path: String,
}

/// The filter entry that names `app`: its Flatpak ref, or its program.
#[must_use]
pub fn key(
    app: &crate::default_apps::Launchable,
    refs: &std::collections::HashMap<String, String>,
) -> Option<String> {
    match &app.flatpak {
        Some(id) => Some(
            refs.get(id)
                .cloned()
                .unwrap_or_else(|| format!("app/{id}/{}/stable", std::env::consts::ARCH)),
        ),
        None => app
            .program
            .as_ref()
            .and_then(|p| p.to_str())
            .map(str::to_owned),
    }
}

/// Installed Flatpak apps' refs (`app/ID/ARCH/BRANCH`) by app ID.
#[must_use]
pub fn flatpak_refs() -> std::collections::HashMap<String, String> {
    let Ok(out) = std::process::Command::new("flatpak")
        .args(["list", "--app", "--columns=application,ref"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return std::collections::HashMap::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let (id, reference) = line.split_once('\t')?;
            let reference = reference.trim();
            (reference.starts_with("app/") && reference.len() <= 512)
                .then(|| (id.trim().to_owned(), reference.to_owned()))
        })
        .take(500)
        .collect()
}

/// What a guardian can change for one account.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Controls {
    pub filter: AppFilter,
    pub allow_installing: bool,
}

/// A program as an app filter names it: the absolute path `PATH` resolves.
#[must_use]
pub fn program_path(program: &str) -> Option<PathBuf> {
    let path = Path::new(program);
    if path.is_absolute() {
        return std::fs::canonicalize(path).ok();
    }
    if program.contains('/') {
        return None;
    }
    std::env::var("PATH")
        .unwrap_or_else(|_| "/usr/bin:/bin".into())
        .split(':')
        .map(|dir| Path::new(dir).join(program))
        .find(|candidate| candidate.is_file())
        .and_then(|found| std::fs::canonicalize(found).ok())
}

static CURRENT: std::sync::RwLock<Option<AppFilter>> = std::sync::RwLock::new(None);

/// The filter the desktop enforces for this account now (read by the shell
/// from AccountsService and kept here for the launcher).
#[must_use]
pub fn current() -> Option<AppFilter> {
    CURRENT.read().ok().and_then(|filter| filter.clone())
}

pub fn set_current(filter: Option<AppFilter>) {
    if let Ok(mut current) = CURRENT.write() {
        *current = filter;
    }
}

/// Whether the current filter lets this account start the desktop entry
/// at `path`.
#[must_use]
pub fn entry_allowed(path: &Path) -> bool {
    let Some(filter) = current() else { return true };
    let (flatpak, program) = crate::default_apps::identity(path);
    filter.allows(flatpak.as_deref(), program.as_deref())
}

/// Whether the current filter lets this account start `program`.
#[must_use]
pub fn program_allowed(program: &Path) -> bool {
    current()
        .is_none_or(|filter| filter.allows(None, std::fs::canonicalize(program).ok().as_deref()))
}

async fn system() -> Result<zbus::Connection, String> {
    zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus").into())
}

async fn properties<'a>(
    connection: &zbus::Connection,
    path: &str,
) -> Result<zbus::Proxy<'a>, String> {
    zbus::Proxy::new(
        connection,
        ACCOUNTS,
        path.to_owned(),
        "org.freedesktop.DBus.Properties",
    )
    .await
    .map_err(|_| tr("shell-accounts-are-unavailable").into())
}

async fn user_path(connection: &zbus::Connection, uid: u32) -> Result<String, String> {
    let manager = zbus::Proxy::new(connection, ACCOUNTS, "/org/freedesktop/Accounts", ACCOUNTS)
        .await
        .map_err(|_| tr("shell-accounts-are-unavailable"))?;
    let path: zbus::zvariant::OwnedObjectPath = manager
        .call("FindUserById", &(i64::from(uid),))
        .await
        .map_err(|_| tr("shell-this-account-is-not-known"))?;
    Ok(path.to_string())
}

fn parse_filter(value: zbus::zvariant::OwnedValue) -> Option<AppFilter> {
    let (allowlist, entries): (bool, Vec<String>) = value.try_into().ok()?;
    let entries = entries
        .into_iter()
        .filter(|e| e.len() <= 4096 && !e.chars().any(char::is_control))
        .take(1024)
        .collect();
    Some(AppFilter { allowlist, entries })
}

/// This account's own filter; `None` where the OS has no parental controls
/// or none are set.
pub async fn own_filter() -> Option<AppFilter> {
    let connection = system().await.ok()?;
    let path = user_path(&connection, rustix::process::getuid().as_raw())
        .await
        .ok()?;
    let value: zbus::zvariant::OwnedValue = properties(&connection, &path)
        .await
        .ok()?
        .call("Get", &(FILTER, "AppFilter"))
        .await
        .ok()?;
    parse_filter(value).filter(AppFilter::restricts)
}

/// Whether this system offers parental controls at all.
pub async fn available() -> bool {
    let Ok(connection) = system().await else {
        return false;
    };
    let Ok(path) = user_path(&connection, rustix::process::getuid().as_raw()).await else {
        return false;
    };
    let Ok(proxy) = properties(&connection, &path).await else {
        return false;
    };
    proxy
        .call::<_, _, zbus::zvariant::OwnedValue>("Get", &(FILTER, "AppFilter"))
        .await
        .is_ok()
}

/// The standard (non-administrator) accounts a guardian can supervise.
pub async fn standard_accounts() -> Result<Vec<Account>, String> {
    let connection = system().await?;
    let manager = zbus::Proxy::new(&connection, ACCOUNTS, "/org/freedesktop/Accounts", ACCOUNTS)
        .await
        .map_err(|_| tr("shell-accounts-are-unavailable"))?;
    let users: Vec<zbus::zvariant::OwnedObjectPath> = manager
        .call("ListCachedUsers", &())
        .await
        .map_err(|_| tr("shell-accounts-are-unavailable"))?;
    let mut found = Vec::new();
    for path in users.into_iter().take(64) {
        let proxy = zbus::Proxy::new(
            &connection,
            ACCOUNTS,
            path.to_string(),
            "org.freedesktop.Accounts.User",
        )
        .await
        .map_err(|_| tr("shell-accounts-are-unavailable"))?;
        // AccountType 0 is a standard account, 1 an administrator.
        let kind: i32 = proxy.get_property("AccountType").await.unwrap_or(1);
        let uid: u64 = proxy.get_property("Uid").await.unwrap_or(0);
        if kind != 0 || uid < 1000 {
            continue;
        }
        let clean = |text: String| {
            text.chars()
                .filter(|c| !c.is_control())
                .take(64)
                .collect::<String>()
        };
        found.push(Account {
            uid: u32::try_from(uid).unwrap_or(u32::MAX),
            name: clean(proxy.get_property("UserName").await.unwrap_or_default()),
            real_name: clean(proxy.get_property("RealName").await.unwrap_or_default()),
            path: path.to_string(),
        });
    }
    Ok(found)
}

/// The controls set for `account` (reading another account's needs the
/// guardian's authorization, which polkit asks for).
pub async fn controls(account: &Account) -> Result<Controls, String> {
    let connection = system().await?;
    let proxy = properties(&connection, &account.path).await?;
    let interactive = zbus::proxy::MethodFlags::AllowInteractiveAuth.into();
    let filter: zbus::zvariant::OwnedValue = proxy
        .call_with_flags("Get", interactive, &(FILTER, "AppFilter"))
        .await
        .map_err(|_| tr("shell-parental-controls-could-not-be-read-an-administrator-must-approve"))?
        .ok_or_else(|| tr("shell-parental-controls-could-not-be-read"))?;
    let installing: Option<zbus::zvariant::OwnedValue> = proxy
        .call_with_flags("Get", interactive, &(FILTER, "AllowUserInstallation"))
        .await
        .ok()
        .flatten();
    Ok(Controls {
        filter: parse_filter(filter).ok_or_else(|| tr("shell-unexpected-parental-controls"))?,
        allow_installing: installing
            .and_then(|v| bool::try_from(v).ok())
            .unwrap_or(true),
    })
}

/// Saves `controls` for `account`; polkit asks an administrator.
pub async fn set_controls(account: &Account, controls: &Controls) -> Result<(), String> {
    if controls.filter.entries.len() > 1024
        || controls
            .filter
            .entries
            .iter()
            .any(|e| !(e.starts_with("app/") || e.starts_with('/')))
    {
        return Err(tr("shell-invalid-app-list").into());
    }
    let connection = system().await?;
    let proxy = properties(&connection, &account.path).await?;
    let interactive = zbus::proxy::MethodFlags::AllowInteractiveAuth.into();
    let filter =
        zbus::zvariant::Value::from((controls.filter.allowlist, controls.filter.entries.clone()));
    proxy
        .call_with_flags::<_, _, ()>("Set", interactive, &(FILTER, "AppFilter", filter))
        .await
        .map_err(|_| tr("shell-the-change-was-not-authorized"))?;
    proxy
        .call_with_flags::<_, _, ()>(
            "Set",
            interactive,
            &(
                FILTER,
                "AllowUserInstallation",
                zbus::zvariant::Value::from(controls.allow_installing),
            ),
        )
        .await
        .map_err(|_| tr("shell-the-change-was-not-authorized"))?;
    Ok(())
}
