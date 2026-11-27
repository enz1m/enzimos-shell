use iced::futures::{SinkExt, StreamExt, channel::mpsc};
use std::{
    collections::{BTreeMap, HashMap},
    time::Duration,
};
use zbus::{
    Connection, MatchRule, Message, MessageStream,
    zvariant::{OwnedObjectPath, OwnedValue},
};

const SERVICE: &str = "org.bluez";
const DEVICE: &str = "org.bluez.Device1";
const BATTERY: &str = "org.bluez.Battery1";
type Properties = HashMap<String, OwnedValue>;
type Interfaces = HashMap<String, Properties>;
type Objects = HashMap<OwnedObjectPath, Interfaces>;
pub(super) type Snapshot = BTreeMap<String, u8>;

pub(super) fn address(value: &str) -> Option<String> {
    (value.len() == 17
        && value.bytes().enumerate().all(|(i, b)| {
            if i % 3 == 2 {
                b == b':'
            } else {
                b.is_ascii_hexdigit()
            }
        }))
    .then(|| value.to_ascii_lowercase())
}

#[allow(
    clippy::iter_over_hash_type,
    reason = "Readings merge by controller address; conflicts become unknown regardless of iteration order."
)]
fn snapshot(objects: &Objects) -> Snapshot {
    let mut reports: BTreeMap<String, Option<u8>> = BTreeMap::new();
    if objects.len() > 512 {
        return Snapshot::new();
    }
    for interfaces in objects.values() {
        let Some(device) = interfaces.get(DEVICE) else {
            continue;
        };
        if device.get("Connected").and_then(|v| bool::try_from(v).ok()) != Some(true) {
            continue;
        }
        let Some(address) = device
            .get("Address")
            .and_then(|v| <&str>::try_from(v).ok())
            .and_then(address)
        else {
            continue;
        };
        let Some(percent) = interfaces
            .get(BATTERY)
            .and_then(|p| p.get("Percentage"))
            .and_then(|v| u8::try_from(v).ok())
            .filter(|v| *v <= 100)
        else {
            continue;
        };
        // Duplicate addresses across adapters must not select an arbitrary report.
        reports
            .entry(address)
            .and_modify(|known| {
                if *known != Some(percent) {
                    *known = None;
                }
            })
            .or_insert(Some(percent));
    }
    reports
        .into_iter()
        .filter_map(|(address, value)| value.map(|value| (address, value)))
        .take(64)
        .collect()
}

fn relevant(message: &Message) -> bool {
    if message.body().len() > 65_536 {
        return false;
    }
    match message
        .header()
        .member()
        .map(zbus::names::MemberName::as_str)
    {
        Some("NameOwnerChanged") => true,
        Some("PropertiesChanged") => {
            let Ok((interface, changed, invalidated)) =
                message
                    .body()
                    .deserialize::<(String, Properties, Vec<String>)>()
            else {
                return false;
            };
            let keys: &[&str] = match interface.as_str() {
                BATTERY => &["Percentage"],
                DEVICE => &["Address", "Connected"],
                _ => return false,
            };
            keys.iter()
                .any(|key| changed.contains_key(*key) || invalidated.iter().any(|name| name == key))
        }
        Some("InterfacesAdded") => message
            .body()
            .deserialize::<(OwnedObjectPath, Interfaces)>()
            .is_ok_and(|(_, interfaces)| {
                interfaces.contains_key(DEVICE) || interfaces.contains_key(BATTERY)
            }),
        Some("InterfacesRemoved") => message
            .body()
            .deserialize::<(OwnedObjectPath, Vec<String>)>()
            .is_ok_and(|(_, interfaces)| {
                interfaces
                    .iter()
                    .any(|name| name == DEVICE || name == BATTERY)
            }),
        _ => false,
    }
}

async fn read(connection: &Connection) -> Snapshot {
    let result = async {
        let manager = zbus::Proxy::new(
            connection,
            SERVICE,
            "/",
            "org.freedesktop.DBus.ObjectManager",
        )
        .await
        .ok()?;
        // Observing controllers must not activate the Bluetooth daemon.
        let objects: Objects = manager
            .call_with_flags(
                "GetManagedObjects",
                zbus::proxy::MethodFlags::NoAutoStart.into(),
                &(),
            )
            .await
            .ok()??;
        Some(snapshot(&objects))
    };
    tokio::time::timeout(Duration::from_secs(3), result)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

async fn follow(
    connection: &Connection,
    output: &mut mpsc::Sender<Snapshot>,
) -> Result<(), zbus::Error> {
    let signal = || MatchRule::builder().msg_type(zbus::message::Type::Signal);
    let properties = signal()
        .sender(SERVICE)?
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .path_namespace("/org/bluez")?
        .build();
    let objects = signal()
        .sender(SERVICE)?
        .interface("org.freedesktop.DBus.ObjectManager")?
        .build();
    let owners = signal()
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .add_arg(SERVICE)?
        .build();
    let properties = MessageStream::for_match_rule(properties, connection, Some(32)).await?;
    let objects = MessageStream::for_match_rule(objects, connection, Some(32)).await?;
    let owners = MessageStream::for_match_rule(owners, connection, Some(8)).await?;
    let mut events =
        iced::futures::stream::select(iced::futures::stream::select(properties, objects), owners);
    let mut last = read(connection).await;
    if output.send(last.clone()).await.is_err() {
        return Ok(());
    }
    let mut deadline = None;
    loop {
        let event = if let Some(at) = deadline {
            match tokio::time::timeout_at(at, events.next()).await {
                Ok(event) => event,
                Err(_) => {
                    deadline = None;
                    let now = read(connection).await;
                    if now != last {
                        last = now.clone();
                        if output.send(now).await.is_err() {
                            return Ok(());
                        }
                    }
                    continue;
                }
            }
        } else {
            events.next().await
        };
        let Some(event) = event else { return Ok(()) };
        if relevant(&event?) && deadline.is_none() {
            deadline = Some(tokio::time::Instant::now() + Duration::from_millis(200));
        }
    }
}

pub(super) fn changes() -> impl iced::futures::Stream<Item = Snapshot> {
    iced::stream::channel(4, async move |mut output: mpsc::Sender<Snapshot>| {
        loop {
            if let Ok(connection) = Connection::system().await {
                let _ = follow(&connection, &mut output).await;
            }
            if output.send(Snapshot::new()).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    })
}
