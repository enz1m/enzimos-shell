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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    fn device(percent: OwnedValue) -> Interfaces {
        HashMap::from([
            (
                DEVICE.into(),
                HashMap::from([
                    (
                        "Address".into(),
                        OwnedValue::from(zbus::zvariant::Str::from("AA:BB:CC:DD:EE:FF")),
                    ),
                    ("Connected".into(), OwnedValue::from(true)),
                ]),
            ),
            (
                BATTERY.into(),
                HashMap::from([("Percentage".into(), percent)]),
            ),
        ])
    }

    fn objects(percent: u8) -> Objects {
        HashMap::from([(
            OwnedObjectPath::try_from("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF").unwrap(),
            device(OwnedValue::from(percent)),
        )])
    }

    struct Manager {
        objects: Arc<Mutex<Objects>>,
        reads: Arc<AtomicUsize>,
    }
    #[zbus::interface(name = "org.freedesktop.DBus.ObjectManager")]
    impl Manager {
        fn get_managed_objects(&self) -> Objects {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.objects.lock().unwrap().clone()
        }
    }

    async fn next(input: &mut mpsc::Receiver<Snapshot>) -> Snapshot {
        tokio::time::timeout(Duration::from_secs(3), input.next())
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn controller_battery_behaviour() {
        // Battery reports require connected exact identity and known byte percentage.
        {
            assert_eq!(
                address("Aa:bB:cc:DD:ee:FF").as_deref(),
                Some("aa:bb:cc:dd:ee:ff")
            );
            for invalid in [
                "",
                "AA_BB_CC_DD_EE_FF",
                "AA:BB:CC:DD:EE:FG",
                "AA:BB:CC:DD:EE:FF\n",
            ] {
                assert!(address(invalid).is_none());
            }
            assert_eq!(snapshot(&objects(0))["aa:bb:cc:dd:ee:ff"], 0);
            assert_eq!(snapshot(&objects(100))["aa:bb:cc:dd:ee:ff"], 100);
            assert!(snapshot(&objects(101)).is_empty());
            let path = OwnedObjectPath::try_from("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF").unwrap();
            assert!(
                snapshot(&HashMap::from([(
                    path.clone(),
                    device(OwnedValue::from(60u32))
                )]))
                .is_empty()
            );
            let mut disconnected = objects(60);
            disconnected
                .get_mut(&path)
                .unwrap()
                .get_mut(DEVICE)
                .unwrap()
                .insert("Connected".into(), OwnedValue::from(false));
            assert!(snapshot(&disconnected).is_empty());
            let mut ambiguous = objects(60);
            ambiguous.insert(
                OwnedObjectPath::try_from("/org/bluez/hci1/dev_AA_BB_CC_DD_EE_FF").unwrap(),
                device(OwnedValue::from(70u8)),
            );
            assert!(snapshot(&ambiguous).is_empty());
        }

        let bus = crate::test_bus::Bus::start();
        let objects = Arc::new(Mutex::new(objects(60)));
        let reads = Arc::new(AtomicUsize::new(0));
        let producer = bus
            .serve(
                SERVICE,
                "/",
                Manager {
                    objects: std::sync::Arc::clone(&objects),
                    reads: std::sync::Arc::clone(&reads),
                },
            )
            .await;
        let listener = bus.client().await;
        let name = listener.unique_name().unwrap().to_owned();
        let (mut output, mut input) = mpsc::channel(4);
        let task = tokio::spawn(async move { follow(&listener, &mut output).await });
        assert_eq!(next(&mut input).await["aa:bb:cc:dd:ee:ff"], 60);
        tokio::time::sleep(Duration::from_millis(350)).await;
        assert_eq!(reads.load(Ordering::SeqCst), 1, "no healthy idle queries");
        for percent in 61..=70u8 {
            *objects.lock().unwrap() = super::tests::objects(percent);
            producer
                .emit_signal(
                    None::<&str>,
                    "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF",
                    "org.freedesktop.DBus.Properties",
                    "PropertiesChanged",
                    &(
                        BATTERY,
                        HashMap::from([("Percentage", OwnedValue::from(percent))]),
                        Vec::<String>::new(),
                    ),
                )
                .await
                .unwrap();
        }
        assert_eq!(next(&mut input).await["aa:bb:cc:dd:ee:ff"], 70);
        assert_eq!(reads.load(Ordering::SeqCst), 2, "one query per burst");
        objects.lock().unwrap().clear();
        producer
            .emit_signal(
                None::<&str>,
                "/",
                "org.freedesktop.DBus.ObjectManager",
                "InterfacesRemoved",
                &(
                    OwnedObjectPath::try_from("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF").unwrap(),
                    vec![BATTERY],
                ),
            )
            .await
            .unwrap();
        assert!(next(&mut input).await.is_empty());
        *objects.lock().unwrap() = super::tests::objects(40);
        producer
            .emit_signal(
                None::<&str>,
                "/",
                "org.freedesktop.DBus.ObjectManager",
                "InterfacesAdded",
                &(
                    OwnedObjectPath::try_from("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF").unwrap(),
                    device(OwnedValue::from(40u8)),
                ),
            )
            .await
            .unwrap();
        assert_eq!(next(&mut input).await["aa:bb:cc:dd:ee:ff"], 40);
        producer.release_name(SERVICE).await.unwrap();
        assert!(next(&mut input).await.is_empty());
        task.abort();
        let _ = task.await;
        let observer = bus.client().await;
        let proxy = zbus::fdo::DBusProxy::new(&observer).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while proxy.name_has_owner(name.clone().into()).await.unwrap() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}
