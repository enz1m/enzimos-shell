//! Native password prompts for the oo7 Secret Service daemon.

use enzui_security::secret::SecretText;
use iced::futures::StreamExt;
use std::{
    collections::HashMap,
    io::Write,
    os::unix::net::UnixStream,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;
use zbus::{
    message::Header,
    zvariant::{OwnedFd, OwnedObjectPath},
};

pub const BUS_NAME: &str = "dev.enzui.SecretPrompter";
const INTERFACE: &str = "org.kde.secretprompter.request";
const MAX_PENDING: usize = 8;

#[derive(Clone, Debug)]
pub enum Event {
    Begin {
        id: u64,
        label: String,
        create: bool,
    },
    Retry {
        id: u64,
    },
    Done {
        id: u64,
    },
}

#[derive(Clone, Debug)]
pub enum Command {
    Answer { id: u64, password: SecretText },
    Cancel { id: u64 },
}

type Pending = Arc<Mutex<HashMap<u64, mpsc::Sender<Command>>>>;
struct Prompter {
    events: mpsc::Sender<Event>,
    pending: Pending,
    next: std::sync::atomic::AtomicU64,
}

impl Prompter {
    async fn begin(
        &self,
        request: OwnedObjectPath,
        label: &str,
        create: bool,
        header: Header<'_>,
        connection: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        let sender = header
            .sender()
            .ok_or_else(|| zbus::fdo::Error::AccessDenied("Missing sender".into()))?
            .to_string();
        let bus = zbus::fdo::DBusProxy::new(connection).await?;
        let owner = bus
            .get_name_owner(
                zbus::names::BusName::try_from("org.freedesktop.secrets")
                    .map_err(zbus::Error::from)?,
            )
            .await?;
        if sender != owner.as_str() || !request.as_str().starts_with("/org/plasma/keyring/Prompt/")
        {
            return Err(zbus::fdo::Error::AccessDenied(
                "Only the credential service may request a prompt".into(),
            ));
        }
        if label.len() > 512 || label.chars().any(char::is_control) {
            return Err(zbus::fdo::Error::InvalidArgs(
                "Invalid keyring label".into(),
            ));
        }
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (commands, receiver) = mpsc::channel(2);
        {
            let mut pending = self
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if pending.len() >= MAX_PENDING {
                return Err(zbus::fdo::Error::LimitsExceeded(
                    "Too many password prompts".into(),
                ));
            }
            pending.insert(id, commands);
        }
        let events = self.events.clone();
        let pending = Arc::clone(&self.pending);
        let connection = connection.clone();
        let label = label.to_owned();
        tokio::spawn(async move {
            let work = conversation(
                &connection,
                &sender,
                request.as_str(),
                Event::Begin { id, label, create },
                &events,
                receiver,
            );
            let accepted = matches!(
                tokio::time::timeout(Duration::from_secs(120), work).await,
                Ok(Ok(true))
            );
            // Cleanup also runs when the daemon disappears or the request times out.
            if let Ok(proxy) =
                zbus::Proxy::new(&connection, sender.as_str(), request.as_str(), INTERFACE).await
            {
                if !accepted {
                    let _ = tokio::time::timeout(
                        Duration::from_secs(2),
                        proxy.call::<_, _, i32>("Rejected", &()),
                    )
                    .await;
                }
                let _ = tokio::time::timeout(
                    Duration::from_secs(2),
                    proxy.call::<_, _, ()>("Dismissed", &()),
                )
                .await;
            }
            pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&id);
            let _ = events.send(Event::Done { id }).await;
        });
        Ok(())
    }
}

#[zbus::interface(name = "org.kde.secretprompter")]
impl Prompter {
    async fn unlock_collection_prompt(
        &self,
        request: OwnedObjectPath,
        window_id: &str,
        activation_token: &str,
        collection_name: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        let _ = (window_id, activation_token);
        self.begin(request, collection_name, false, header, connection)
            .await
    }
    async fn create_collection_prompt(
        &self,
        request: OwnedObjectPath,
        window_id: &str,
        activation_token: &str,
        collection_name: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        let _ = (window_id, activation_token);
        self.begin(request, collection_name, true, header, connection)
            .await
    }
}

async fn conversation(
    connection: &zbus::Connection,
    sender: &str,
    path: &str,
    event: Event,
    events: &mpsc::Sender<Event>,
    mut commands: mpsc::Receiver<Command>,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let Event::Begin { id, .. } = event else {
        return Err("a prompt request is required".into());
    };
    let proxy = zbus::Proxy::new(connection, sender, path, INTERFACE).await?;
    let mut dismiss = proxy.receive_signal("Dismiss").await?;
    events.send(event).await?;
    let mut owner_check = tokio::time::interval(Duration::from_secs(1));
    let bus = zbus::fdo::DBusProxy::new(connection).await?;
    loop {
        let command = tokio::select! {
            _ = dismiss.next() => break,
            _ = owner_check.tick() => {
                let owner = bus.get_name_owner(zbus::names::BusName::try_from("org.freedesktop.secrets")?).await;
                if !owner.is_ok_and(|owner| owner.as_str() == sender) { break; }
                continue;
            },
            command = commands.recv() => command,
        };
        match command {
            Some(Command::Answer { password, .. }) => {
                if password.is_empty() || password.expose_secret().len() > 4096 {
                    continue;
                }
                let (reader, mut writer) = UnixStream::pair()?;
                reader.set_nonblocking(true)?;
                writer.write_all(password.expose_secret().as_bytes())?;
                writer.shutdown(std::net::Shutdown::Write)?;
                drop(password);
                let fd = OwnedFd::from(std::os::fd::OwnedFd::from(reader));
                let result: i32 =
                    tokio::time::timeout(Duration::from_secs(10), proxy.call("Accepted", &(fd,)))
                        .await??;
                if result == 0 {
                    return Ok(true);
                }
                events.send(Event::Retry { id }).await?;
            }
            Some(Command::Cancel { .. }) | None => break,
        }
    }
    Ok(false)
}

pub async fn run(
    events: mpsc::Sender<Event>,
    commands: mpsc::Receiver<Command>,
) -> Result<(), String> {
    serve(
        zbus::connection::Builder::session().map_err(|e| e.to_string())?,
        events,
        commands,
    )
    .await
}
async fn serve(
    builder: zbus::connection::Builder<'_>,
    events: mpsc::Sender<Event>,
    mut commands: mpsc::Receiver<Command>,
) -> Result<(), String> {
    let pending: Pending = Arc::default();
    let _connection = builder
        .name(BUS_NAME)
        .map_err(|e| e.to_string())?
        .serve_at(
            "/SecretPrompter",
            Prompter {
                events,
                pending: Arc::clone(&pending),
                next: std::sync::atomic::AtomicU64::new(1),
            },
        )
        .map_err(|e| e.to_string())?
        .build()
        .await
        .map_err(|e| e.to_string())?;
    while let Some(command) = commands.recv().await {
        let id = match &command {
            Command::Answer { id, .. } | Command::Cancel { id } => *id,
        };
        let target = pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&id)
            .cloned();
        if let Some(target) = target {
            let _ = target.try_send(command);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, Read},
        process::{Child, Stdio},
    };
    struct Bus(Child);
    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    struct Callback {
        seen: Arc<Mutex<Vec<Vec<u8>>>>,
        rejected: Arc<std::sync::atomic::AtomicBool>,
    }
    #[zbus::interface(name = "org.kde.secretprompter.request")]
    impl Callback {
        fn accepted(&self, fd: OwnedFd) -> i32 {
            let mut file = std::fs::File::from(std::os::fd::OwnedFd::from(fd));
            let mut value = Vec::new();
            file.read_to_end(&mut value).unwrap();
            let correct = value == b"fixture-correct";
            self.seen.lock().unwrap().push(value);
            if correct { 0 } else { 1 }
        }
        fn rejected(&self) -> i32 {
            self.rejected
                .store(true, std::sync::atomic::Ordering::SeqCst);
            0
        }
        fn dismissed(&self) {}
    }
    #[tokio::test]
    async fn keyring_prompt_routes_secrets_only_to_the_owner_and_recovers_from_retry_and_cancel() {
        let mut child = std::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut address = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let _bus = Bus(child);
        let address = address.trim();
        let path = "/org/plasma/keyring/Prompt/p1";
        let seen = Arc::default();
        let rejected = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let daemon = zbus::connection::Builder::address(address)
            .unwrap()
            .name("org.freedesktop.secrets")
            .unwrap()
            .serve_at(
                path,
                Callback {
                    seen: Arc::clone(&seen),
                    rejected: Arc::clone(&rejected),
                },
            )
            .unwrap()
            .build()
            .await
            .unwrap();
        let (events, mut incoming) = mpsc::channel(16);
        let (commands, receiver) = mpsc::channel(16);
        let server = tokio::spawn(serve(
            zbus::connection::Builder::address(address).unwrap(),
            events,
            receiver,
        ));
        let client = zbus::connection::Builder::address(address)
            .unwrap()
            .build()
            .await
            .unwrap();
        let bus = zbus::fdo::DBusProxy::new(&client).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !bus
                .name_has_owner(zbus::names::BusName::try_from(BUS_NAME).unwrap())
                .await
                .unwrap()
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let args = (OwnedObjectPath::try_from(path).unwrap(), "", "", "Login");
        let rogue = zbus::Proxy::new(
            &client,
            BUS_NAME,
            "/SecretPrompter",
            "org.kde.secretprompter",
        )
        .await
        .unwrap();
        assert!(
            rogue
                .call::<_, _, ()>("UnlockCollectionPrompt", &args)
                .await
                .is_err()
        );
        assert!(
            incoming.try_recv().is_err(),
            "an unrelated app must not open password prompts"
        );
        let owner = zbus::Proxy::new(
            &daemon,
            BUS_NAME,
            "/SecretPrompter",
            "org.kde.secretprompter",
        )
        .await
        .unwrap();
        owner
            .call::<_, _, ()>("UnlockCollectionPrompt", &args)
            .await
            .unwrap();
        let Some(Event::Begin { id, create, .. }) = incoming.recv().await else {
            panic!("missing prompt");
        };
        assert!(!create);
        commands
            .send(Command::Answer {
                id,
                password: SecretText::new("fixture-wrong".into()),
            })
            .await
            .unwrap();
        assert!(matches!(incoming.recv().await, Some(Event::Retry { id: retry }) if retry == id));
        commands
            .send(Command::Answer {
                id,
                password: SecretText::new("fixture-correct".into()),
            })
            .await
            .unwrap();
        assert!(matches!(incoming.recv().await, Some(Event::Done { id: done }) if done == id));
        assert_eq!(
            *seen.lock().unwrap(),
            [b"fixture-wrong".to_vec(), b"fixture-correct".to_vec()]
        );
        assert!(
            !rejected.load(std::sync::atomic::Ordering::SeqCst),
            "a successful unlock must not be cancelled afterwards"
        );
        owner
            .call::<_, _, ()>("CreateCollectionPrompt", &args)
            .await
            .unwrap();
        let Some(Event::Begin { id, create, .. }) = incoming.recv().await else {
            panic!("missing create prompt");
        };
        assert!(create);
        commands.send(Command::Cancel { id }).await.unwrap();
        assert!(matches!(incoming.recv().await, Some(Event::Done { id: done }) if done == id));
        assert!(rejected.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(
            seen.lock().unwrap().len(),
            2,
            "cancellation must send no password"
        );
        server.abort();
        let _ = server.await;
    }
}
