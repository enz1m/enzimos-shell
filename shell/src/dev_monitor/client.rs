use super::*;
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use std::fmt::Write as _;

#[derive(Clone, Debug)]
pub struct Client {
    connection: zbus::Connection,
    owner: String,
}

impl Client {
    pub async fn session() -> Result<Self, String> {
        let connection = tokio::time::timeout(REQUEST_TIMEOUT, zbus::Connection::session())
            .await
            .map_err(|_| tr("shell-session-bus-connection-timed-out").to_owned())?
            .map_err(|e| e.to_string())?;
        Self::connect(connection).await
    }

    async fn connect(connection: zbus::Connection) -> Result<Self, String> {
        tokio::time::timeout(REQUEST_TIMEOUT, async {
            version(&connection, NAME).await?;
            let bus = zbus::fdo::DBusProxy::new(&connection)
                .await
                .map_err(|e| e.to_string())?;
            let name = NAME
                .try_into()
                .map_err(|e: zbus::names::Error| e.to_string())?;
            let owner = bus
                .get_name_owner(name)
                .await
                .map_err(|e| e.to_string())?
                .to_string();
            version(&connection, &owner).await?;
            if bus
                .get_name_owner(
                    NAME.try_into()
                        .map_err(|e: zbus::names::Error| e.to_string())?,
                )
                .await
                .map_err(|e| e.to_string())?
                .as_str()
                != owner
            {
                return Err(tr("shell-dev-monitor-owner-changed-during-connection").into());
            }
            Ok(Self { connection, owner })
        })
        .await
        .map_err(|_| tr("shell-dev-monitor-activation-timed-out").to_owned())?
    }

    pub fn authority_key(&self) -> &str {
        &self.owner
    }

    pub async fn snapshot(&self, options: Options) -> Result<Snapshot, String> {
        let json: String = self
            .call(
                "Snapshot",
                &(options.system, options.udp, options.containers),
            )
            .await?;
        Snapshot::parse(&json)
    }

    pub async fn shortcuts(&self, target: Target) -> Result<Shortcuts, String> {
        let text: String = match &target {
            Target::Process(id) if id.pid > 1 && id.start_time > 0 => {
                self.call("Shortcuts", &(id.pid, id.start_time)).await?
            }
            Target::Container(id) if id.valid() => {
                self.call("ContainerShortcuts", &(&id.engine, &id.socket, &id.id))
                    .await?
            }
            _ => {
                return Err(tr("shell-invalid-developer-shortcut-identity").into());
            }
        };
        Shortcuts::parse(&text, &target)
    }
    pub async fn inspect(&self, id: ProcessId) -> Result<Detail, String> {
        if id.pid <= 1 || id.start_time == 0 {
            return Err(tr("shell-invalid-process-identity").into());
        }
        let text: String = self.call("Inspect", &(id.pid, id.start_time)).await?;
        Detail::parse(&text, id)
    }
    pub async fn lookup(&self, port: u16) -> Result<Lookup, String> {
        if port == 0 {
            return Err(tr("shell-enter-a-port-from-1-to-65535").into());
        }
        let text: String = self.call("Lookup", &(port,)).await?;
        Lookup::parse(&text, port)
    }

    pub async fn action(&self, target: Target, operation: Operation) -> Result<String, String> {
        match target {
            Target::Process(id) => {
                if id.pid <= 1
                    || id.start_time == 0
                    || !matches!(operation, Operation::Stop | Operation::ForceStop)
                {
                    return Err(tr("shell-invalid-process-action").into());
                }
                let json: String = self
                    .call(
                        "Signal",
                        &(
                            id.pid,
                            id.start_time,
                            true,
                            operation == Operation::ForceStop,
                        ),
                    )
                    .await?;
                let report: SignalReport = parse(&json)?;
                let mut result = tr!("shell-signal-delivered-to-process-es", "arg0" => report.signal.clone(), "arg1" => format!("{:}", report.delivered.len()));
                for skipped in report.skipped {
                    let _ = write!(result, "\nPID {}: {}", skipped.pid, skipped.reason);
                }
                Ok(result)
            }
            Target::Container(id) => {
                if !id.valid() || operation == Operation::ForceStop {
                    return Err(tr("shell-invalid-container-action").into());
                }
                let action = match operation {
                    Operation::Start => "start",
                    Operation::Stop => "stop",
                    Operation::Restart => "restart",
                    Operation::ForceStop => unreachable!(),
                };
                let _: () = self
                    .call("Container", &(id.engine, id.socket, id.id, action))
                    .await?;
                Ok(
                    tr!("shell-request-completed-waiting-for-inventory-readback", "arg0" => operation.label().to_owned()),
                )
            }
        }
    }

    pub async fn logs(&self, target: Target) -> Result<String, String> {
        let text = match target {
            Target::Process(id) => {
                let json: String = self
                    .call("ProcessLogs", &(id.pid, id.start_time, 200u32))
                    .await?;
                #[derive(Deserialize)]
                struct Logs {
                    text: String,
                    note: Option<String>,
                }
                let logs: Logs = parse(&json)?;
                match logs.note {
                    Some(note) => format!("{note}\n\n{}", logs.text),
                    None => logs.text,
                }
            }
            Target::Container(id) => {
                if !id.valid() {
                    return Err(tr("shell-invalid-container-identity").into());
                }
                self.call("ContainerLogs", &(id.engine, id.socket, id.id, 200u32))
                    .await?
            }
        };
        if text.len() > 256 * 1024 + 4096 {
            return Err(tr("shell-dev-monitor-log-exceeds-the-display-limit").into());
        }
        Ok(text)
    }

    async fn call<B, R>(&self, method: &str, body: &B) -> Result<R, String>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
        R: serde::de::DeserializeOwned + zbus::zvariant::Type,
    {
        let reply = tokio::time::timeout(REQUEST_TIMEOUT, async {
            let bus = zbus::fdo::DBusProxy::new(&self.connection).await?;
            let name = NAME.try_into().map_err(zbus::Error::from)?;
            if bus.get_name_owner(name).await?.as_str() != self.owner {
                return Err(zbus::Error::Failure(
                    tr("shell-dev-monitor-owner-changed-reopen-this-view-before-continuing").into(),
                ));
            }
            self.connection
                .call_method(Some(self.owner.as_str()), PATH, Some(NAME), method, body)
                .await
        })
        .await
        .map_err(|_| {
            tr("shell-dev-monitor-request-timed-out-outcome-may-be-unknown-it-will-not-be-retried")
                .to_owned()
        })?
        .map_err(|e| e.to_string())?;
        reply.body().deserialize().map_err(|e| e.to_string())
    }
}

async fn version(connection: &zbus::Connection, owner: &str) -> Result<(), String> {
    let reply = connection
        .call_method(
            Some(owner),
            PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(NAME, "Version"),
        )
        .await
        .map_err(|e| e.to_string())?;
    let value: zbus::zvariant::OwnedValue =
        reply.body().deserialize().map_err(|e| e.to_string())?;
    let version = u32::try_from(value).map_err(|e| e.to_string())?;
    if version != VERSION {
        return Err(
            tr!("shell-unsupported-dev-monitor-version-2", "arg0" => format!("{:}", version)),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Fixture {
        version: u32,
        calls: Arc<Mutex<Vec<String>>>,
        refuse: bool,
    }

    #[zbus::interface(name = "org.enzimos.DevMonitor1")]
    impl Fixture {
        #[zbus(property)]
        fn version(&self) -> u32 {
            self.version
        }
        fn snapshot(&self, system: bool, udp: bool, containers: bool) -> String {
            self.calls
                .lock()
                .expect("fixture request log mutex is not poisoned")
                .push(format!("snapshot:{system}:{udp}:{containers}"));
            "{\"version\":2,\"scan_id\":1,\"taken_at_unix_ms\":1,\"ports\":[],\"processes\":[],\"engines\":[],\"warnings\":[]}".into()
        }
        fn shortcuts(&self, pid: u32, start: u64) -> String {
            self.calls
                .lock()
                .unwrap()
                .push(format!("shortcuts:{pid}:{start}"));
            format!(
                "{{\"pid\":{pid},\"start_time\":{start},\"directory\":\"/tmp/project\",\"folder_uri\":\"file:///tmp/project\",\"folder\":{{\"argv\":[\"/usr/bin/xdg-open\",\"file:///tmp/project\"],\"directory\":\"/tmp/project\"}},\"terminal\":{{\"argv\":[\"/usr/bin/terminal-emulator\",\"--working-directory\",\"/tmp/project\"],\"directory\":\"/tmp/project\"}}}}"
            )
        }
        fn container_shortcuts(&self, engine: String, socket: String, id: String) -> String {
            self.calls
                .lock()
                .unwrap()
                .push(format!("container-shortcuts:{engine}:{socket}:{id}"));
            "{\"directory\":\"/tmp/project\",\"folder_uri\":\"file:///tmp/project\",\"folder\":{\"argv\":[\"/usr/bin/xdg-open\",\"file:///tmp/project\"],\"directory\":\"/tmp/project\"},\"terminal\":{\"argv\":[\"/usr/bin/terminal-emulator\",\"--working-directory\",\"/tmp/project\"],\"directory\":\"/tmp/project\"}}".into()
        }
        fn inspect(&self, pid: u32, start: u64) -> String {
            self.calls
                .lock()
                .unwrap()
                .push(format!("inspect:{pid}:{start}"));
            format!(
                "{{\"pid\":{pid},\"start_time\":{start},\"label\":\"web\",\"cmdline\":[],\"threads\":1,\"children\":[]}}"
            )
        }
        fn lookup(&self, port: u16) -> String {
            self.calls.lock().unwrap().push(format!("lookup:{port}"));
            "{\"ports\":[],\"processes\":[],\"warnings\":[]}".into()
        }
        fn signal(
            &self,
            pid: u32,
            start: u64,
            tree: bool,
            force: bool,
        ) -> zbus::fdo::Result<String> {
            self.calls
                .lock()
                .map_err(|_| zbus::fdo::Error::Failed("Fixture lock poisoned".into()))?
                .push(format!("signal:{pid}:{start}:{tree}:{force}"));
            if self.refuse {
                return Err(zbus::fdo::Error::Failed("stale process identity".into()));
            }
            Ok(format!(
                "{{\"signal\":\"SIGTERM\",\"delivered\":[{pid}],\"skipped\":[{{\"pid\":43,\"reason\":\"protected branch\"}}]}}"
            ))
        }
        fn container(&self, engine: String, socket: String, id: String, action: String) {
            self.calls
                .lock()
                .expect("fixture request log mutex is not poisoned")
                .push(format!("container:{engine}:{socket}:{id}:{action}"));
        }
    }

    #[tokio::test]
    async fn client_async_behaviour() {
        // Private bus preserves displayed identity and does not retry refusals.
        {
            let bus = crate::test_bus::Bus::start();
            let calls = Arc::new(Mutex::new(Vec::new()));
            let _service = bus
                .serve(
                    NAME,
                    PATH,
                    Fixture {
                        version: 2,
                        calls: std::sync::Arc::clone(&calls),
                        refuse: true,
                    },
                )
                .await;
            let client = Client::connect(bus.client().await).await.unwrap();
            client.snapshot(Options::default()).await.unwrap();
            let error = client
                .action(
                    Target::Process(ProcessId {
                        pid: 42,
                        start_time: 123,
                    }),
                    Operation::Stop,
                )
                .await
                .unwrap_err();
            assert!(error.contains("stale process identity"));
            assert_eq!(
                *calls.lock().unwrap(),
                ["snapshot:false:false:false", "signal:42:123:true:false"]
            );
            client
                .action(
                    Target::Process(ProcessId {
                        pid: 42,
                        start_time: 123,
                    }),
                    Operation::ForceStop,
                )
                .await
                .unwrap_err();
            assert_eq!(
                calls.lock().unwrap().last().unwrap(),
                "signal:42:123:true:true"
            );
            client
                .action(
                    Target::Container(ContainerId {
                        engine: "podman".into(),
                        socket: "/run/test.sock".into(),
                        id: "a".repeat(64),
                        name: "friendly".into(),
                    }),
                    Operation::Restart,
                )
                .await
                .unwrap();
            assert!(
                calls
                    .lock()
                    .unwrap()
                    .last()
                    .unwrap()
                    .ends_with(&format!(":{}:restart", "a".repeat(64)))
            );
        }
        // A client does not send old actions to a replacement service owner.
        {
            let bus = crate::test_bus::Bus::start();
            let old_calls = Arc::new(Mutex::new(Vec::new()));
            let service = bus
                .serve(
                    NAME,
                    PATH,
                    Fixture {
                        version: 2,
                        calls: std::sync::Arc::clone(&old_calls),
                        refuse: true,
                    },
                )
                .await;
            let client = Client::connect(bus.client().await).await.unwrap();
            service.release_name(NAME).await.unwrap();
            let new_calls = Arc::new(Mutex::new(Vec::new()));
            let _replacement = bus
                .serve(
                    NAME,
                    PATH,
                    Fixture {
                        version: 2,
                        calls: std::sync::Arc::clone(&new_calls),
                        refuse: false,
                    },
                )
                .await;
            assert!(
                client
                    .action(
                        Target::Process(ProcessId {
                            pid: 42,
                            start_time: 123
                        }),
                        Operation::Stop
                    )
                    .await
                    .is_err()
            );
            assert!(old_calls.lock().unwrap().is_empty());
            assert!(new_calls.lock().unwrap().is_empty());
            service.close().await.unwrap();
        }
    }
}
