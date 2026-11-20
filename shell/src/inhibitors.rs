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
    crate::bus::bounded(list_now())
        .await
        .unwrap_or_else(|| Err(tr("shell-logind-did-not-list-inhibitors").to_owned()))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn lock(what: &str, mode: &str, uid: u32) -> Inhibitor {
        Inhibitor {
            who: "App".into(),
            why: "Busy".into(),
            what: what.into(),
            mode: mode.into(),
            uid,
        }
    }

    #[test]
    fn only_blocking_shutdown_locks_hold_off_power_actions() {
        let me = rustix::process::getuid().as_raw();
        let all = [
            lock("shutdown:sleep", "block", 0),
            lock("shutdown", "delay", 0),
            lock("sleep", "block", me),
            lock("idle", "block", me + 1),
        ];
        assert_eq!(relevant(&all, false), vec![all[0].clone()]);
        assert_eq!(relevant(&all, true), vec![all[2].clone()]);
    }
}
