use super::{
    Compositor, Peer, Result,
    eis::Eis,
    generation,
    indicator::Indicator,
    portal::{BACKEND, Portal},
    read_frame, socket_path, unavailable, valid_id, write_frame,
};
use iced::futures::StreamExt;
use serde_json::json;
use std::{
    cell::RefCell,
    fs,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::{Semaphore, watch},
};

const CHECK: Duration = Duration::from_millis(250);
const IO_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub(super) struct Cancel(watch::Sender<bool>);

impl Cancel {
    fn new() -> Self {
        Self(watch::channel(false).0)
    }

    fn allowed(&self) -> bool {
        !*self.0.borrow()
    }

    pub(super) fn revoke(&self) {
        self.0.send_replace(true);
    }

    async fn revoked(&self) {
        let mut state = self.0.subscribe();
        let _ = state.wait_for(|revoked| *revoked).await;
    }
}

struct Grant {
    session: String,
    generation: String,
    cancel: Cancel,
}

type Slot = Rc<RefCell<Option<Grant>>>;

pub(super) fn run(compositor: Compositor) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    tokio::task::LocalSet::new().block_on(&runtime, serve(compositor))
}

async fn serve(compositor: Compositor) -> Result<()> {
    let path = socket_path().ok_or_else(unavailable)?;
    let uid = rustix::process::getuid().as_raw();
    let base = path.parent().ok_or_else(unavailable)?;
    if !fs::symlink_metadata(base)
        .is_ok_and(|m| m.is_dir() && m.uid() == uid && m.mode() & 0o077 == 0)
    {
        return Err(unavailable());
    }
    if let Ok(meta) = fs::symlink_metadata(&path) {
        if !meta.file_type().is_socket()
            || meta.uid() != uid
            || UnixStream::connect(&path).await.is_ok()
        {
            return Err(unavailable());
        }
        fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    let listener = UnixListener::bind(&path).map_err(|e| e.to_string())?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    let shell = std::env::current_exe().map_err(|e| e.to_string())?;
    let assistant = shell.with_file_name("enzui-assistant");
    let slot = Slot::default();
    let limit = Arc::new(Semaphore::new(8));
    loop {
        let (stream, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let Ok(permit) = Arc::clone(&limit).try_acquire_owned() else {
            continue;
        };
        let Some(peer) = Peer::new(&stream, &[&shell, &assistant]) else {
            continue;
        };
        let (slot, compositor) = (Rc::clone(&slot), compositor.clone());
        tokio::task::spawn_local(async move {
            let _permit = permit;
            client(stream, peer, slot, compositor).await;
        });
    }
}

async fn client(stream: UnixStream, peer: Peer, slot: Slot, compositor: Compositor) {
    let mut socket = BufReader::new(stream);
    let reply = async {
        let request = tokio::time::timeout(IO_TIMEOUT, read_frame(&mut socket))
            .await
            .map_err(|_| unavailable())??;
        let session = request["session_id"]
            .as_str()
            .filter(|id| valid_id(id))
            .ok_or_else(unavailable)?;
        if !peer.alive() {
            return Err(unavailable());
        }
        match request["method"].as_str() {
            Some("revoke") => {
                if let Some(grant) = slot
                    .borrow()
                    .as_ref()
                    .filter(|grant| grant.session == session)
                {
                    grant.cancel.revoke();
                }
                Ok(())
            }
            Some("allow") if slot.borrow().is_none() => {
                let generation = generation()?;
                let cancel = Cancel::new();
                *slot.borrow_mut() = Some(Grant {
                    session: session.to_owned(),
                    generation: generation.clone(),
                    cancel: cancel.clone(),
                });
                let (status, ready) = tokio::sync::oneshot::channel();
                let session = session.to_owned();
                let owner = Rc::clone(&slot);
                let worker = cancel.clone();
                tokio::task::spawn_local(async move {
                    if let Err(error) =
                        lifetime(&compositor, &generation, &session, worker.clone(), status).await
                        && worker.allowed()
                    {
                        eprintln!("enzui-shell: desktop control: {error}");
                    }
                    worker.revoke();
                    if owner
                        .borrow()
                        .as_ref()
                        .is_some_and(|grant| grant.generation == generation)
                    {
                        *owner.borrow_mut() = None;
                    }
                });
                tokio::select! {
                    biased;
                    _ = socket.read_u8() => {
                        cancel.revoke();
                        Err(unavailable())
                    }
                    result = ready => result.map_err(|_| unavailable()),
                }
            }
            _ => Err(unavailable()),
        }
    }
    .await;
    let _ = tokio::time::timeout(
        IO_TIMEOUT,
        write_frame(socket.get_mut(), &json!({"ok": reply.is_ok()})),
    )
    .await;
}

async fn compositor_check(compositor: &Compositor, state: &str, generation: &str) -> Result<()> {
    let command = format!("enzuiassistant {state} {generation}");
    compositor_command(compositor, &command).await
}

async fn compositor_command(compositor: &Compositor, command: &str) -> Result<()> {
    let response = tokio::time::timeout(IO_TIMEOUT, compositor.request(&[command]))
        .await
        .map_err(|_| unavailable())??;
    if response.trim() == "ok" {
        Ok(())
    } else {
        Err(unavailable())
    }
}

async fn watch_compositor(compositor: &Compositor, generation: &str, state: &str) -> Result<()> {
    let mut interval = tokio::time::interval(CHECK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        compositor_check(compositor, state, generation).await?;
    }
}

async fn compositor_events(compositor: &Compositor, generation: &str) -> Result<()> {
    let socket = UnixStream::connect(compositor.event_socket().ok_or_else(unavailable)?)
        .await
        .map_err(|_| unavailable())?;
    let mut socket = BufReader::new(socket);
    let expected = format!("enzuiassistantrevoked>>{generation}\n");
    loop {
        use tokio::io::AsyncBufReadExt;
        let mut line = Vec::new();
        (&mut socket)
            .take(65537)
            .read_until(b'\n', &mut line)
            .await
            .map_err(|_| unavailable())?;
        if !line.ends_with(b"\n")
            || line.len() > 65536
            || line == expected.as_bytes()
            || line.starts_with(b"monitoradded>>")
            || line.starts_with(b"monitorremoved>>")
        {
            return Err(unavailable());
        }
    }
}

async fn lifetime(
    compositor: &Compositor,
    generation: &str,
    session_id: &str,
    cancel: Cancel,
    status: tokio::sync::oneshot::Sender<()>,
) -> Result<()> {
    let portal = tokio::time::timeout(IO_TIMEOUT, Portal::new(generation))
        .await
        .map_err(|_| unavailable())??;
    let arm = format!("enzuiassistant arm {generation} {}", portal.handle());
    if let Err(error) = compositor_command(compositor, &arm).await {
        let _ = compositor_check(compositor, "disarm", generation).await;
        return Err(error);
    }
    let result = tokio::select! {
        biased;
        () = cancel.revoked() => Ok(()),
        result = compositor_events(compositor, generation) => result,
        result = control(compositor, generation, session_id, &cancel, &portal, status) => result,
    };
    cancel.revoke();
    let _ = compositor_check(compositor, "disarm", generation).await;
    portal.close().await;
    result
}

async fn control(
    compositor: &Compositor,
    generation: &str,
    session_id: &str,
    cancel: &Cancel,
    portal: &Portal,
    status: tokio::sync::oneshot::Sender<()>,
) -> Result<()> {
    let (mut harness, peer) =
        tokio::time::timeout(IO_TIMEOUT, attach_harness(session_id, generation))
            .await
            .map_err(|_| unavailable())??;
    let bus = zbus::fdo::DBusProxy::new(&portal.connection)
        .await
        .map_err(|_| unavailable())?;
    let mut owners = bus
        .receive_name_owner_changed_with_args(&[(0, BACKEND)])
        .await
        .map_err(|_| unavailable())?;
    let session = portal.session().await?;
    let mut closed = session
        .receive_signal("Closed")
        .await
        .map_err(|_| unavailable())?;
    let pending = async {
        portal.start().await?;
        let backend = zbus::names::BusName::try_from(BACKEND).map_err(|_| unavailable())?;
        if !bus
            .name_has_owner(backend)
            .await
            .map_err(|_| unavailable())?
        {
            return Err(unavailable());
        }
        let (indicator, shown) = Indicator::start(cancel.clone());
        tokio::time::timeout(IO_TIMEOUT, shown)
            .await
            .map_err(|_| unavailable())?
            .map_err(|_| unavailable())?;
        compositor_check(compositor, "visible", generation).await?;
        let mut eis = Eis::new(portal.connect_eis().await?.into())?;
        tokio::time::timeout(IO_TIMEOUT, async {
            while !eis.ready() {
                eis.dispatch().await?;
            }
            Ok::<(), String>(())
        })
        .await
        .map_err(|_| unavailable())??;
        compositor_check(compositor, "scoped", generation).await?;
        Ok::<_, String>((indicator, eis))
    };
    let (_indicator, mut eis) = tokio::select! {
        biased;
        _ = closed.next() => return Err(unavailable()),
        _ = harness.read_u8() => return Err(unavailable()),
        result = watch_compositor(compositor, generation, "pending") => return result,
        result = tokio::time::timeout(Duration::from_secs(65), pending) => result.map_err(|_| unavailable())??,
    };
    tokio::time::timeout(IO_TIMEOUT, async {
        write_frame(
            harness.get_mut(),
            &json!({"generation":generation,"activate":true}),
        )
        .await?;
        let reply = read_frame(&mut harness).await?;
        if reply["ok"] != true || reply["generation"] != generation || !peer.alive() {
            return Err(unavailable());
        }
        Ok(())
    })
    .await
    .map_err(|_| unavailable())??;
    let backend_lost = async {
        while let Some(change) = owners.next().await {
            let change = change.args().map_err(|_| unavailable())?;
            if change.old_owner().as_ref().is_some() {
                break;
            }
        }
        Err(unavailable())
    };
    status.send(()).map_err(|()| unavailable())?;
    tokio::select! {
        biased;
        _ = closed.next() => Err(unavailable()),
        result = backend_lost => result,
        result = watch_compositor(compositor, generation, "scoped") => result,
        result = async {
            loop {
                let mut request = {
                    let frame = read_frame(&mut harness);
                    tokio::pin!(frame);
                    loop {
                        tokio::select! {
                            biased;
                            result = eis.dispatch() => result?,
                            result = &mut frame => break result?,
                        }
                    }
                };
                if request["generation"] != generation || !peer.alive() || !cancel.allowed() {
                    return Err(unavailable());
                }
                compositor_check(compositor, "visible", generation).await?;
                let arguments = request["arguments"].take();
                let name = request["tool"].as_str().ok_or_else(unavailable)?;
                let ok = eis.perform(name, arguments, || cancel.allowed())?;
                tokio::time::timeout(IO_TIMEOUT,
                    write_frame(harness.get_mut(), &json!({"generation":generation,"ok":ok})))
                    .await.map_err(|_| unavailable())??;
            }
        } => result,
    }
}

async fn attach_harness(session: &str, generation: &str) -> Result<(BufReader<UnixStream>, Peer)> {
    let path = std::env::var_os("ENZUI_AGENT_SOCKET")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(crate::agent_harness::socket_path)
        .ok_or_else(unavailable)?;
    let socket = UnixStream::connect(&path)
        .await
        .map_err(|_| unavailable())?;
    let installed = Path::new("/usr/bin/enzimos-agent-harness");
    let preview = crate::desktop_session::mode()
        .nested()
        .then(|| std::env::current_exe().map(|path| path.with_file_name("enzimos-agent-harness")))
        .transpose()
        .map_err(|e| e.to_string())?;
    let peer = Peer::new(
        &socket,
        &[installed, preview.as_deref().unwrap_or(installed)],
    )
    .ok_or_else(unavailable)?;
    let mut socket = BufReader::new(socket);
    write_frame(
        socket.get_mut(),
        &json!({
            "schema_version": 1, "method": "session.desktop_control",
            "session_id": session, "generation": generation,
        }),
    )
    .await?;
    let reply = tokio::time::timeout(IO_TIMEOUT, read_frame(&mut socket))
        .await
        .map_err(|_| unavailable())??;
    if reply["ok"] != true || reply["generation"] != generation || !peer.alive() {
        return Err(unavailable());
    }
    Ok((socket, peer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn revocation_is_terminal_even_before_the_indicator_is_ready() {
        let revoked = Cancel::new();
        let next = Cancel::new();
        revoked.revoke();
        revoked.revoked().await;
        assert!(!revoked.allowed());
        assert!(next.allowed());
    }
}
