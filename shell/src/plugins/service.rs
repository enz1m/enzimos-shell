use super::{
    budget::BudgetPool,
    manifest::{Manifest, Package},
    protocol::{self, Output, Request, Snapshot},
    store::{Record, Registry, Store},
    worker::{Fault, Worker},
};
use enzui_shell::i18n::tr;
use iced::futures::{SinkExt, Stream};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub id: String,
    pub activation: u64,
    pub run: u64,
}
#[derive(Clone)]
pub enum Command {
    Input {
        owner: Owner,
        element: String,
        revision: u64,
        action: String,
        value: Option<bool>,
        gesture: String,
    },
    Reply {
        owner: Owner,
        id: u64,
        result: Value,
    },
    Provider {
        owner: Owner,
        data: Value,
    },
    Refresh,
}
#[derive(Clone)]
pub enum Event {
    Connected(mpsc::Sender<Command>),
    Unavailable(String),
    Available,
    Catalog(u64),
    Started {
        owner: Owner,
        manifest: Arc<Manifest>,
        authority: Arc<AtomicBool>,
    },
    Snapshot {
        owner: Owner,
        manifest: Arc<Manifest>,
        snapshot: Arc<Snapshot>,
    },
    Removed(Owner),
    Request {
        owner: Owner,
        id: u64,
        request: Request,
    },
    Fault {
        owner: Owner,
        fault: Fault,
    },
}
impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Connected(_) => "PluginConnected",
            Self::Unavailable(_) => "PluginUnavailable",
            Self::Available => "PluginAvailable",
            Self::Catalog(_) => "PluginCatalog",
            Self::Started { .. } => "PluginStarted",
            Self::Snapshot { .. } => "PluginSnapshot",
            Self::Removed(_) => "PluginRemoved",
            Self::Request { .. } => "PluginRequest",
            Self::Fault { .. } => "PluginFault",
        })
    }
}
#[derive(Clone, Hash)]
struct Config {
    root: PathBuf,
}
pub fn subscription(root: PathBuf) -> iced::Subscription<Event> {
    iced::Subscription::run_with(Config { root }, stream)
}
fn stream(config: &Config) -> impl Stream<Item = Event> + use<> {
    let root = config.root.clone();
    iced::stream::channel(32, async move |mut output| {
        let (tx, rx) = mpsc::channel(32);
        if output.send(Event::Connected(tx)).await.is_err() {
            return;
        }
        if let Err(reason) = run(root, rx, output.clone()).await {
            let _ = output.send(Event::Unavailable(reason)).await;
        }
        std::future::pending::<()>().await;
    })
}
type Sink = iced::futures::channel::mpsc::Sender<Event>;
enum ActorCommand {
    Input {
        element: String,
        revision: u64,
        action: String,
        value: Option<bool>,
        gesture: String,
    },
    Reply {
        id: u64,
        result: Value,
    },
    Provider(Value),
    Config(Value),
}
enum ActorEvent {
    Output { owner: Owner, output: Output },
    Fault { owner: Owner, fault: Fault },
}
struct Running {
    owner: Owner,
    record: Record,
    manifest: Arc<Manifest>,
    commands: mpsc::Sender<ActorCommand>,
    task: Option<JoinHandle<()>>,
    shutdown: Option<oneshot::Sender<()>>,
    authority: Arc<AtomicBool>,
}
impl Drop for Running {
    fn drop(&mut self) {
        self.request_stop();
    }
}
impl Running {
    fn request_stop(&mut self) {
        self.authority.store(false, Ordering::Release);
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(mut task) = self.task.take() {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    if tokio::time::timeout(Duration::from_secs(4), &mut task)
                        .await
                        .is_err()
                    {
                        task.abort();
                        let _ = task.await;
                    }
                });
            } else {
                task.abort();
            }
        }
    }
    fn stop(mut self) {
        self.request_stop();
    }
}
fn classify(fault: Fault) -> &'static str {
    match fault {
        Fault::Unavailable => "unavailable",
        Fault::Exited => "exited",
        Fault::Deadline | Fault::InputBlocked => "unresponsive",
        Fault::Flood | Fault::FrameTooLarge => "resource-limit",
        Fault::InvalidJson | Fault::Protocol => "invalid-data",
    }
}
fn start_actor(
    pool: BudgetPool,
    package: Package,
    record: Record,
    owner: Owner,
    events: mpsc::Sender<ActorEvent>,
) -> Running {
    let (tx, mut commands) = mpsc::channel(16);
    let (shutdown, mut stopping) = oneshot::channel();
    let manifest = Arc::new(package.manifest().clone());
    let actor_owner = owner.clone();
    let actor_record = record.clone();
    let task = tokio::spawn(async move {
        let result=async {
            let mut worker=tokio::select!{biased; _=&mut stopping=>return Ok(()),worker=Worker::spawn(&pool,&package,actor_record.config(),actor_record.grants())=>worker?};
            let mut sequence = 0u64;
            let mut pong_deadline = None;
            let mut needs_ping = true;
            let mut latest: Option<Arc<Snapshot>> = None;
            let mut request_id = 0u64;
            let result = loop {
                let result = tokio::select! {
                    _ = &mut stopping => break Ok(()),
                    command = commands.recv() => {
                        let frame = match command {
                            None => break Ok(()),
                            Some(ActorCommand::Input { element, revision, action, value, gesture }) => {
                                if !latest.as_ref().is_some_and(|ui| {
                                    ui.revision == revision && ui.permits_action(&element, &action, value)
                                }) {
                                    continue;
                                }
                                json!({"type":"action","element":element,"revision":revision,"action":action,"value":value,"gesture":gesture})
                            }
                            Some(ActorCommand::Reply { id, result }) => json!({"type":"response","id":id,"result":result}),
                            Some(ActorCommand::Provider(data)) => json!({"type":"provider","data":data}),
                            Some(ActorCommand::Config(config)) => json!({"type":"config","config":config}),
                        };
                        needs_ping = true;
                        worker.send(&frame).await
                    }
                    () = std::future::ready(()), if needs_ping && pong_deadline.is_none() => async {
                        sequence = sequence.checked_add(1).ok_or(Fault::Protocol)?;
                        worker.send(&json!({"type":"ping","sequence":sequence})).await?;
                        pong_deadline = Some(tokio::time::Instant::now() + Duration::from_secs(3));
                        needs_ping = false;
                        Ok(())
                    }.await,
                    () = async { tokio::time::sleep_until(pong_deadline.expect("pending ping")).await },
                        if pong_deadline.is_some() => Err(Fault::Deadline),
                    frame=worker.receive_idle()=>async {
                        let frame=frame?;let output=protocol::decode(&frame,package.manifest(),actor_record.grants()).map_err(|_|Fault::Protocol)?;
                        match output {
                            Output::Ready=>Ok(()),
                            Output::Pong(got) if got == sequence && pong_deadline.is_some() => {
                                pong_deadline = None;
                                Ok(())
                            }
                            Output::Pong(_) => Err(Fault::Protocol),
                            Output::Snapshot(snapshot)=>{
                                if latest.as_ref().is_some_and(|old|snapshot.revision<=old.revision){Err(Fault::Protocol)}else{
                                    latest=Some(Arc::new(snapshot.clone()));events.send(ActorEvent::Output{owner:actor_owner.clone(),output:Output::Snapshot(snapshot)}).await.map_err(|_|Fault::Exited)
                                }
                            },
                            Output::Request{id,request}=>{if id<=request_id{Err(Fault::Protocol)}else{request_id=id;events.send(ActorEvent::Output{owner:actor_owner.clone(),output:Output::Request{id,request}}).await.map_err(|_|Fault::Exited)}},
                        }
                    }.await
                };
                if let Err(error)=result{break Err(error);}
            };
            worker.shutdown().await?;result
        }.await;
        if let Err(fault) = result {
            let _ = events
                .send(ActorEvent::Fault {
                    owner: actor_owner,
                    fault,
                })
                .await;
        }
    });
    Running {
        owner,
        record,
        manifest,
        commands: tx,
        task: Some(task),
        shutdown: Some(shutdown),
        authority: Arc::new(AtomicBool::new(true)),
    }
}
async fn emit(output: &mut Sink, event: Event) -> Result<(), String> {
    output
        .send(event)
        .await
        .map_err(|_| tr("shell-plugin-ui-consumer-ended").into())
}
async fn fault(
    store: &Store,
    actors: &mut BTreeMap<String, Running>,
    blocked: &mut BTreeMap<String, u64>,
    output: &mut Sink,
    owner: Owner,
    reason: Fault,
) -> Result<(), String> {
    if !actors.get(&owner.id).is_some_and(|a| a.owner == owner) {
        return Ok(());
    }
    let actor = actors
        .remove(&owner.id)
        .expect("the exclusively borrowed actor map still contains the selected owner");
    let digest = actor.record.digest();
    blocked.insert(owner.id.clone(), owner.activation);
    actor.stop();
    emit(output, Event::Removed(owner.clone())).await?;
    emit(
        output,
        Event::Fault {
            owner: owner.clone(),
            fault: reason,
        },
    )
    .await?;
    let store = store.clone();
    let _ = tokio::task::spawn_blocking(move || {
        store.quarantine(&owner.id, digest, owner.activation, classify(reason))
    })
    .await;
    Ok(())
}
async fn sync(
    store: &Store,
    pool: &BudgetPool,
    actors: &mut BTreeMap<String, Running>,
    blocked: &mut BTreeMap<String, u64>,
    next: &mut u64,
    events: &mpsc::Sender<ActorEvent>,
    output: &mut Sink,
) -> Result<u64, String> {
    pool.reap();
    let read = store.clone();
    let Registry { entries, serial } = tokio::task::spawn_blocking(move || read.registry())
        .await
        .map_err(|_| tr("shell-plugin-state-reader-ended"))??;
    blocked.retain(|id, _| entries.iter().any(|r| r.id() == id));
    let removed: Vec<_> = actors
        .iter()
        .filter(|(id, a)| {
            !entries.iter().any(|r| {
                r.id() == id.as_str()
                    && r.enabled()
                    && r.activation() == a.owner.activation
                    && r.digest() == a.record.digest()
            })
        })
        .map(|(id, _)| id.clone())
        .collect();
    for id in removed {
        let actor = actors
            .remove(&id)
            .expect("the exclusively borrowed actor map still contains the selected owner");
        let owner = actor.owner.clone();
        actor.stop();
        emit(output, Event::Removed(owner)).await?;
    }
    for record in entries.into_iter().filter(Record::enabled) {
        if blocked.get(record.id()) == Some(&record.activation()) {
            continue;
        }
        if let Some(actor) = actors.get_mut(record.id()) {
            if record.config_revision() != actor.record.config_revision() {
                if actor
                    .commands
                    .try_send(ActorCommand::Config(record.config().clone()))
                    .is_err()
                {
                    let owner = actor.owner.clone();
                    fault(store, actors, blocked, output, owner, Fault::InputBlocked).await?;
                } else {
                    actor.record = record;
                }
            }
            continue;
        }
        let read = store.clone();
        let selected = record.clone();
        let package = tokio::task::spawn_blocking(move || read.package(&selected))
            .await
            .map_err(|_| tr("shell-plugin-package-reader-ended"))?;
        let Ok(package) = package else {
            blocked.insert(record.id().to_owned(), record.activation());
            let owner = Owner {
                id: record.id().to_owned(),
                activation: record.activation(),
                run: *next,
            };
            let selected = owner.clone();
            let read = store.clone();
            let digest = record.digest();
            let _ = tokio::task::spawn_blocking(move || {
                read.quarantine(&selected.id, digest, selected.activation, "unavailable")
            })
            .await;
            emit(
                output,
                Event::Fault {
                    owner,
                    fault: Fault::Unavailable,
                },
            )
            .await?;
            continue;
        };
        *next = next
            .checked_add(1)
            .ok_or_else(|| tr("shell-plugin-execution-identities-exhausted"))?;
        let owner = Owner {
            id: record.id().to_owned(),
            activation: record.activation(),
            run: *next,
        };
        let manifest = Arc::new(package.manifest().clone());
        let started = owner.clone();
        let actor = start_actor(pool.clone(), package, record, owner, events.clone());
        let authority = std::sync::Arc::clone(&actor.authority);
        actors.insert(started.id.clone(), actor);
        emit(
            output,
            Event::Started {
                owner: started,
                manifest,
                authority,
            },
        )
        .await?;
    }
    Ok(serial)
}
fn initialized_store(
    root: PathBuf,
    settings: Option<crate::settings::Store>,
) -> Result<Store, String> {
    match settings.filter(|settings| {
        settings
            .path()
            .parent()
            .is_some_and(|parent| parent.join("plugins") == root)
    }) {
        Some(settings) => Store::for_settings(settings),
        None => Store::at(root),
    }
}
async fn run(
    root: PathBuf,
    mut commands: mpsc::Receiver<Command>,
    mut output: Sink,
) -> Result<(), String> {
    let pool = BudgetPool::from_environment()?;
    let store = initialized_store(root, crate::settings::Store::from_environment())?;
    let (events, mut messages) = mpsc::channel(32);
    let mut actors = BTreeMap::new();
    let mut blocked = BTreeMap::new();
    let mut next = 0;
    let mut unavailable = false;
    let mut refresh = true;
    let mut defaults_ready = false;
    // The shell's folder watch sends Refresh when the store changes; the timer
    // is only a safety net, for a store the watch cannot see.
    let fallback = Duration::from_secs(60);
    let mut timer = tokio::time::interval_at(tokio::time::Instant::now() + fallback, fallback);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        if refresh {
            let result = async {
                if !defaults_ready {
                    let initialize = store.clone();
                    tokio::task::spawn_blocking(move || {
                        initialize.initialize_defaults()?;
                        initialize.refresh_bundled_clipboard()
                    })
                    .await
                    .map_err(|_| tr("shell-default-plugin-initialization-stopped").to_owned())??;
                    defaults_ready = true;
                }
                sync(
                    &store,
                    &pool,
                    &mut actors,
                    &mut blocked,
                    &mut next,
                    &events,
                    &mut output,
                )
                .await
            }
            .await;
            match result {
                Ok(serial) => {
                    emit(&mut output, Event::Catalog(serial)).await?;
                    if unavailable {
                        emit(&mut output, Event::Available).await?;
                    }
                    unavailable = false;
                }
                Err(error) => {
                    actors.clear();
                    blocked.clear();
                    if !unavailable {
                        emit(&mut output, Event::Unavailable(error)).await?;
                    }
                    unavailable = true;
                }
            }
            refresh = false;
        }
        tokio::select! {
            _=timer.tick()=>refresh=true,
            command=commands.recv()=>match command {
                None=>return Ok(()),
                Some(Command::Refresh)=>refresh=true,
                Some(command)=>{
                    let(owner,command)=match command {
                        Command::Input{owner,element,revision,action,value,gesture}=>(owner,ActorCommand::Input{element,revision,action,value,gesture}),
                        Command::Reply{owner,id,result}=>(owner,ActorCommand::Reply{id,result}),
                        Command::Provider{owner,data}=>(owner,ActorCommand::Provider(data)),Command::Refresh=>unreachable!(),
                    };
                    if let Some(actor)=actors.get(&owner.id).filter(|a|a.owner==owner)
                        && actor.commands.try_send(command).is_err(){fault(&store,&mut actors,&mut blocked,&mut output,owner,Fault::InputBlocked).await?;}
                }
            },
            message=messages.recv()=>match message {
                None=>return Ok(()),
                Some(ActorEvent::Fault{owner,fault:reason})=>fault(&store,&mut actors,&mut blocked,&mut output,owner,reason).await?,
                Some(ActorEvent::Output{owner,output:event})=>{
                    if let Some(actor)=actors.get(&owner.id).filter(|a|a.owner==owner){
                        let event=match event {Output::Snapshot(snapshot)=>Event::Snapshot{owner,manifest:std::sync::Arc::clone(&actor.manifest),snapshot:Arc::new(snapshot)},Output::Request{id,request}=>Event::Request{owner,id,request},_=>continue};
                        emit(&mut output,event).await?;
                    }
                }
            }
        }
    }
}
