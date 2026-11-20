//! Cancellable polkit approval for agent-requested computer settings.
//! Mutation follows approval on the same bus connection and never opens a second interactive
//! prompt.

use enzui_shell::i18n::tr;
use std::collections::HashMap;
use zbus::zvariant::Value;

const AUTHORITY: &str = "org.freedesktop.PolicyKit1";
const TIMEDATE: &str = "org.freedesktop.timedate1";
// polkit AllowUserInteraction flag.
const INTERACTIVE: u32 = 1;

/// A change to the computer's clock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Timezone(String),
    Ntp(bool),
}
impl Change {
    #[must_use]
    pub fn action(&self) -> &'static str {
        match self {
            Self::Timezone(_) => "org.freedesktop.timedate1.set-timezone",
            Self::Ntp(_) => "org.freedesktop.timedate1.set-ntp",
        }
    }
}

/// How asking for approval ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Approval {
    Approved,
    Refused,
    /// Cancelled before an answer; nothing changed.
    Withdrawn,
}

/// One system bus connection: polkit cancels a check only for the
/// connection that started it.
#[derive(Clone, Debug)]
pub struct Authority {
    connection: zbus::Connection,
}

impl Authority {
    pub async fn connect() -> Result<Self, String> {
        Ok(Self {
            connection: zbus::Connection::system()
                .await
                .map_err(|_| tr("shell-no-system-bus"))?,
        })
    }

    /// Asks for `action`, showing the session's password prompt, until it is
    /// answered or `withdraw` resolves (then the prompt is cancelled).
    pub async fn approve(
        &self,
        action: &str,
        cancellation: &str,
        withdraw: impl std::future::Future<Output = ()>,
    ) -> Result<Approval, String> {
        let name = self
            .connection
            .unique_name()
            .ok_or_else(|| tr("shell-no-bus-name"))?
            .to_string();
        let subject: (&str, HashMap<&str, Value<'_>>) = (
            "system-bus-name",
            HashMap::from([("name", Value::from(name.as_str()))]),
        );
        let details: HashMap<&str, &str> = HashMap::new();
        let proxy = zbus::Proxy::new(
            &self.connection,
            AUTHORITY,
            "/org/freedesktop/PolicyKit1/Authority",
            "org.freedesktop.PolicyKit1.Authority",
        )
        .await
        .map_err(|_| tr("shell-polkit-is-unavailable"))?;
        let arguments = (subject, action, details, INTERACTIVE, cancellation);
        let check = proxy
            .call::<_, _, (bool, bool, HashMap<String, String>)>("CheckAuthorization", &arguments);
        tokio::select! {
            result = check => match result {
                Ok((true, _, _)) => Ok(Approval::Approved),
                Ok((false, _, _)) => Ok(Approval::Refused),
                Err(zbus::Error::MethodError(name, _, _)) if name.as_str() == "org.freedesktop.PolicyKit1.Error.Cancelled" => Ok(Approval::Withdrawn),
                Err(_) => Err(tr("shell-the-approval-could-not-be-asked-for").into()),
            },
            () = withdraw => {
                let _ = proxy.call::<_, _, ()>("CancelCheckAuthorization", &(cancellation,)).await;
                Ok(Approval::Withdrawn)
            }
        }
    }

    /// Makes `change` without asking again. A refusal is a public code and a
    /// message.
    pub async fn commit(&self, change: &Change) -> Result<(), (&'static str, String)> {
        let proxy = zbus::Proxy::new(
            &self.connection,
            TIMEDATE,
            "/org/freedesktop/timedate1",
            TIMEDATE,
        )
        .await
        .map_err(|_| {
            (
                "failed",
                tr("shell-date-and-time-settings-are-unavailable").to_owned(),
            )
        })?;
        let result = match change {
            Change::Timezone(zone) => {
                proxy
                    .call::<_, _, ()>("SetTimezone", &(zone.as_str(), false))
                    .await
            }
            Change::Ntp(on) => proxy.call::<_, _, ()>("SetNTP", &(*on, false)).await,
        };
        result.map_err(|error| match error {
            zbus::Error::MethodError(name, _, _) if name.as_str() == "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired" => {
                ("not_authorized", tr("shell-this-computer-does-not-keep-an-administrator-s-approval-so-it-was-not-change").to_owned())
            }
            zbus::Error::MethodError(_, Some(message), _) => ("failed", message.chars().filter(|c| !c.is_control()).take(200).collect()),
            _ => ("failed", tr("shell-the-setting-could-not-be-changed").to_owned()),
        })
    }
}
