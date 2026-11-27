//! logind shutdown inhibitors displayed before power-action confirmation.

use enzui_shell::i18n::tr;
#[derive(Clone, Debug, PartialEq, Eq)]

pub struct Inhibitor {
    pub who: String,
    pub why: String,
    pub what: String,
    /// "block" stops the action; "delay" only postpones it briefly.
    pub mode: String,
    pub uid: u32,
}

impl Inhibitor {
    #[must_use]
    pub fn blocks_shutdown(&self) -> bool {
        self.mode == "block" && self.what.split(':').any(|what| what == "shutdown")
    }
}

/// The relevant locks for an action: shutdown locks for power off and
/// restart; for logging out, this account's own block locks.
#[must_use]
pub fn relevant(all: &[Inhibitor], logout: bool) -> Vec<Inhibitor> {
    let uid = rustix::process::getuid().as_raw();
    all.iter()
        .filter(|lock| lock.mode == "block")
        .filter(|lock| {
            if logout {
                lock.uid == uid
            } else {
                lock.blocks_shutdown()
            }
        })
        .cloned()
        .collect()
}

pub async fn list() -> Result<Vec<Inhibitor>, String> {
    let (logind, portal) = tokio::join!(
        crate::bus::bounded(list_now()),
        crate::desktop_portal::portal_inhibit::session_inhibitors(false),
    );
    let mut locks = match logind {
        Some(Ok(locks)) => locks,
        _ if !portal.is_empty() => Vec::new(),
        _ => return Err(tr("shell-logind-did-not-list-inhibitors").into()),
    };
    locks.extend(portal.into_iter().map(|(who, why)| Inhibitor {
        who: who.chars().filter(|c| !c.is_control()).take(80).collect(),
        why: why.chars().filter(|c| !c.is_control()).take(80).collect(),
        what: "logout".into(),
        mode: "block".into(),
        uid: rustix::process::getuid().as_raw(),
    }));
    Ok(locks)
}

async fn list_now() -> Result<Vec<Inhibitor>, String> {
    let system = crate::bus::system()
        .await
        .ok_or_else(|| tr("shell-no-system-bus"))?;
    let manager = zbus::Proxy::new(
        &system,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await
    .map_err(|_| tr("shell-logind-is-not-available"))?;
    let rows: Vec<(String, String, String, String, u32, u32)> = manager
        .call("ListInhibitors", &())
        .await
        .map_err(|_| tr("shell-logind-did-not-list-inhibitors"))?;
    let clean = |text: String| {
        text.chars()
            .filter(|c| !c.is_control())
            .take(80)
            .collect::<String>()
    };
    Ok(rows
        .into_iter()
        .take(32)
        .map(|(what, who, why, mode, uid, _pid)| Inhibitor {
            who: clean(who),
            why: clean(why),
            what,
            mode,
            uid,
        })
        .collect())
}
