use iced::futures::{SinkExt, StreamExt, channel::mpsc};
use std::{
    collections::{BTreeMap, HashMap},
    time::Duration,
};
use zbus::{Connection, MatchRule, Message, MessageStream, zvariant::OwnedValue};

const LIMIT: usize = 64;
const INTERFACE: &str = "com.canonical.Unity.LauncherEntry";
const COALESCE: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Badge {
    pub count: u64,
    pub count_visible: bool,
    pub urgent: bool,
    pub progress: Option<u8>,
    progress_visible: bool,
}
impl Badge {
    pub fn label(self) -> Option<String> {
        (self.count_visible && self.count > 0).then(|| {
            if self.count > 99 {
                "99+".into()
            } else {
                self.count.to_string()
            }
        })
    }
}
pub type Snapshot = BTreeMap<String, Badge>;

pub fn desktop_id(uri: &str) -> Option<&str> {
    let id = uri.strip_prefix("application://")?;
    (id.len() > 8
        && id.len() <= 255
        && id.ends_with(".desktop")
        && id.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)))
    .then_some(id)
}
pub fn group_id(group: &crate::dock::Group) -> Option<String> {
    if let Some(id) = group
        .id
        .strip_prefix("app:desktop/")
        .or_else(|| group.id.strip_prefix("app:flatpak/"))
    {
        return desktop_id(&format!("application://{id}")).map(str::to_owned);
    }
    match group.app.as_ref()? {
        crate::search::AppTarget::Prepared(id) => {
            crate::catalog::desktop_id(id).map(|id| format!("{id}.desktop"))
        }
        _ => None,
    }
}
#[derive(Default)]
struct State {
    entries: BTreeMap<(String, String), Badge>,
}
#[derive(Default)]
struct Patch {
    count: Option<u64>,
    visible: Option<bool>,
    urgent: Option<bool>,
    progress: Option<u8>,
    progress_visible: Option<bool>,
}
fn patch(properties: &HashMap<String, OwnedValue>) -> Option<Patch> {
    if properties.len() > 32 {
        return None;
    }
    let count = match properties.get("count") {
        Some(value) => Some(u64::try_from(i64::try_from(value).ok()?).ok()?),
        None => None,
    };
    let visible = properties
        .get("count-visible")
        .map(bool::try_from)
        .transpose()
        .ok()?;
    let urgent = properties
        .get("urgent")
        .map(bool::try_from)
        .transpose()
        .ok()?;
    let progress = properties
        .get("progress")
        .and_then(|value| f64::try_from(value).ok())
        .filter(|value| (0.0..=1.0).contains(value))
        .map(|value| (value * 100.0).round() as u8);
    let progress_visible = properties
        .get("progress-visible")
        .and_then(|value| bool::try_from(value).ok());
    (count.is_some()
        || visible.is_some()
        || urgent.is_some()
        || progress.is_some()
        || progress_visible.is_some())
    .then_some(Patch {
        count,
        visible,
        urgent,
        progress,
        progress_visible,
    })
}
impl State {
    fn contains_sender(&self, sender: &str) -> bool {
        self.entries.keys().any(|(owner, _)| owner == sender)
    }
    fn update(&mut self, sender: &str, id: &str, patch: Patch) -> bool {
        let key = (sender.to_owned(), id.to_owned());
        if !self.entries.contains_key(&key) && self.entries.len() >= LIMIT {
            return false;
        }
        let badge = self.entries.entry(key).or_default();
        let previous = *badge;
        if let Some(count) = patch.count {
            badge.count = count;
        }
        if let Some(visible) = patch.visible {
            badge.count_visible = visible;
        }
        if let Some(urgent) = patch.urgent {
            badge.urgent = urgent;
        }
        if let Some(progress) = patch.progress {
            badge.progress = Some(progress);
        }
        if let Some(visible) = patch.progress_visible {
            badge.progress_visible = visible;
            if !visible {
                badge.progress = None;
            }
        }
        *badge != previous
    }
    fn remove(&mut self, sender: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(owner, _), _| owner != sender);
        self.entries.len() != before
    }
    fn snapshot(&self) -> Snapshot {
        let mut result: Snapshot = BTreeMap::new();
        let mut progress_publishers = std::collections::BTreeSet::new();
        for ((_, id), badge) in &self.entries {
            let progress = badge.progress.filter(|_| badge.progress_visible);
            if !badge.urgent && (!badge.count_visible || badge.count == 0) && progress.is_none() {
                continue;
            }
            let value = result.entry(id.clone()).or_default();
            value.urgent |= badge.urgent;
            if progress.is_some() {
                value.progress = if progress_publishers.insert(id) {
                    progress
                } else {
                    None
                };
            }
            if badge.count_visible {
                value.count = value.count.max(badge.count);
                value.count_visible = true;
            }
        }
        result
    }
}
async fn apply(state: &mut State, bus: &zbus::fdo::DBusProxy<'_>, message: Message) -> bool {
    if message.body().len() > 8192 {
        return false;
    }
    let header = message.header();
    match header.member().map(zbus::names::MemberName::as_str) {
        Some("NameOwnerChanged") => {
            let Ok((name, old, new)) = message.body().deserialize::<(String, String, String)>()
            else {
                return false;
            };
            name.starts_with(':') && name == old && new.is_empty() && state.remove(&name)
        }
        Some("Update") => {
            let Some(sender) = header.sender() else {
                return false;
            };
            let Ok((uri, properties)) = message
                .body()
                .deserialize::<(String, HashMap<String, OwnedValue>)>()
            else {
                return false;
            };
            let Some(id) = desktop_id(&uri) else {
                return false;
            };
            let Some(patch) = patch(&properties) else {
                return false;
            };
            // Independent match streams may deliver a queued update after owner loss.
            // Check new senders instead of resurrecting their stale state.
            if !state.contains_sender(sender.as_str())
                && !bus
                    .name_has_owner(sender.clone().into())
                    .await
                    .unwrap_or(false)
            {
                return false;
            }
            state.update(sender.as_str(), id, patch)
        }
        _ => false,
    }
}
async fn follow(
    connection: &Connection,
    generation: u64,
    output: &mut mpsc::Sender<(u64, Snapshot)>,
) -> Result<(), zbus::Error> {
    let updates = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(INTERFACE)?
        .member("Update")?
        .build();
    let owners = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .build();
    let updates = MessageStream::for_match_rule(updates, connection, Some(64)).await?;
    let owners = MessageStream::for_match_rule(owners, connection, Some(64)).await?;
    let bus = zbus::fdo::DBusProxy::new(connection).await?;
    let mut events = iced::futures::stream::select(updates, owners);
    if output.send((generation, Snapshot::new())).await.is_err() {
        return Ok(());
    }
    let mut state = State::default();
    let mut published = Snapshot::new();
    let mut deadline = None;
    loop {
        let event = if let Some(at) = deadline {
            match tokio::time::timeout_at(at, events.next()).await {
                Ok(event) => event,
                Err(_) => {
                    deadline = None;
                    let snapshot = state.snapshot();
                    if snapshot != published {
                        if output.send((generation, snapshot.clone())).await.is_err() {
                            return Ok(());
                        }
                        published = snapshot;
                    }
                    continue;
                }
            }
        } else {
            events.next().await
        };
        let Some(event) = event else {
            return Ok(());
        };
        if apply(&mut state, &bus, event?).await && deadline.is_none() {
            deadline = Some(tokio::time::Instant::now() + COALESCE);
        }
    }
}
pub fn subscription(generation: u64) -> iced::Subscription<(u64, Snapshot)> {
    iced::Subscription::run_with(generation, stream)
}
#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "Subscription::run_with borrows the concrete hashed input."
)]
fn stream(generation: &u64) -> impl iced::futures::Stream<Item = (u64, Snapshot)> + use<> {
    let generation = *generation;
    iced::stream::channel(1, async move |mut output| {
        loop {
            if let Ok(connection) = Connection::session().await {
                let _ = follow(&connection, generation, &mut output).await;
            }
            if output.send((generation, Snapshot::new())).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    })
}
