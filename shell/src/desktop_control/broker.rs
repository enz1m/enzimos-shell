use super::{
    Compositor, Failure, Peer, Result,
    eis::Eis,
    executable, generation,
    indicator::Indicator,
    portal::{BACKEND, Portal},
    read_frame, report, socket_path, unavailable, valid_id, write_frame,
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

type Attempt<T> = std::result::Result<T, Failure>;

const CHECK: Duration = Duration::from_millis(250);
const IO_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub(super) struct Cancel(watch::Sender<Option<Attempt<()>>>);

impl Cancel {
    fn new() -> Self {
        Self(watch::channel(None).0)
    }

    fn allowed(&self) -> bool {
        self.0.borrow().is_none()
    }

    pub(super) fn revoke(&self) {
        self.stop(Ok(()));
    }

    pub(super) fn indicator_failed(&self) {
        self.stop(Err(("indicator mapping", "Wayland indicator failed")));
    }

    fn stop(&self, result: Attempt<()>) {
        self.0.send_if_modified(|current| {
            if current.is_some() {
                return false;
            }
            *current = Some(result);
            true
        });
    }

    async fn revoked(&self) -> Attempt<()> {
        let mut state = self.0.subscribe();
        (*state
            .wait_for(Option::is_some)
            .await
            .expect("cancel sender is live"))
        .expect("wait_for observes revocation")
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
    let shell = executable("enzui-shell").map_err(|e| e.to_string())?;
    let assistant = executable("enzui-assistant").map_err(|e| e.to_string())?;
    let slot = Slot::default();
    let limit = Arc::new(Semaphore::new(8));
    loop {
        let (stream, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let Ok(permit) = Arc::clone(&limit).try_acquire_owned() else {
            report(("request slot", "too many clients"));
            let _ = stream.try_write(b"{\"ok\":false}\n");
            continue;
        };
        let peer = match Peer::new(&stream, &[&shell, &assistant]) {
            Ok(peer) => peer,
            Err(reason) => {
                report(("client peer check", reason));
                let _ = stream.try_write(b"{\"ok\":false}\n");
                continue;
            }
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
            .map_err(|_| ("request frame", "request timed out"))?
            .map_err(|_| ("request frame", "invalid or incomplete request"))?;
        let session = request["session_id"]
            .as_str()
            .filter(|id| valid_id(id))
            .ok_or(("session ID", "missing or invalid conversation ID"))?;
        if !peer.alive() {
            return Err(("client peer check", "peer exited before request"));
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
                Ok(true)
            }
            Some("allow") if slot.borrow().is_none() => {
                let generation =
                    generation().map_err(|_| ("request slot", "could not allocate a lease ID"))?;
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
                    {
                        report(error);
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
                        Err(("client exchange", "client disconnected before consent completed"))
                    }
                    result = ready => Ok(result.is_ok()),
                }
            }
            Some("allow") => Err((
                "request slot",
                "another control request is pending or active",
            )),
            _ => Err(("request frame", "unknown request method")),
        }
    }
    .await;
    let ok = reply.unwrap_or_else(|failure| {
        report(failure);
        false
    });
    if !matches!(
        tokio::time::timeout(
            IO_TIMEOUT,
            write_frame(socket.get_mut(), &json!({"ok": ok}))
        )
        .await,
        Ok(Ok(()))
    ) && ok
    {
        report(("client exchange", "could not send broker response"));
    }
}

async fn compositor_check(compositor: &Compositor, state: &str, generation: &str) -> Attempt<()> {
    let command = format!("enzuiassistant {state} {generation}");
    let stage = match state {
        "visible" => "indicator mapping",
        "scoped" => "compositor input route",
        "pending" => "compositor pending lease",
        _ => "compositor cleanup",
    };
    compositor_command(compositor, &command, stage).await
}

async fn compositor_command(
    compositor: &Compositor,
    command: &str,
    stage: &'static str,
) -> Attempt<()> {
    let response = tokio::time::timeout(IO_TIMEOUT, compositor.request(&[command]))
        .await
        .map_err(|_| (stage, "compositor timed out"))?
        .map_err(|error| {
            let reason = if error.contains("not the registered shell") {
                "Shell is not registered with the compositor"
            } else if error.contains("session locked") {
                "desktop session is locked"
            } else if error.contains("assistant control revoked") {
                "lease has been revoked"
            } else if error.contains("assistant indicator or input route unavailable") {
                "compositor could not verify indicator or input route"
            } else {
                "compositor request failed"
            };
            (stage, reason)
        })?;
    if response.trim() == "ok" {
        Ok(())
    } else {
        Err((stage, "unexpected response from compositor"))
    }
}

async fn watch_compositor(compositor: &Compositor, generation: &str, state: &str) -> Attempt<()> {
    let mut interval = tokio::time::interval(CHECK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        compositor_check(compositor, state, generation).await?;
    }
}

async fn compositor_events(compositor: &Compositor, generation: &str) -> Attempt<()> {
    let stage = "compositor events";
    let path = compositor
        .event_socket()
        .ok_or((stage, "event socket is missing"))?;
    let socket = UnixStream::connect(path)
        .await
        .map_err(|_| (stage, "cannot connect to event socket"))?;
    let mut socket = BufReader::new(socket);
    let expected = format!("enzuiassistantrevoked>>{generation}\n");
    loop {
        use tokio::io::AsyncBufReadExt;
        let mut line = Vec::new();
        (&mut socket)
            .take(65537)
            .read_until(b'\n', &mut line)
            .await
            .map_err(|_| (stage, "cannot read event socket"))?;
        if !line.ends_with(b"\n") || line.len() > 65536 {
            return Err((stage, "event socket closed or sent an invalid frame"));
        }
        if line == expected.as_bytes() {
            return Err((stage, "compositor revoked desktop control"));
        }
        if line.starts_with(b"monitoradded>>") || line.starts_with(b"monitorremoved>>") {
            return Err((stage, "display configuration changed"));
        }
    }
}

async fn lifetime(
    compositor: &Compositor,
    generation: &str,
    session_id: &str,
    cancel: Cancel,
    status: tokio::sync::oneshot::Sender<()>,
) -> Attempt<()> {
    let portal = tokio::time::timeout(IO_TIMEOUT, Portal::new(generation))
        .await
        .map_err(|_| ("portal session bus", "connection timed out"))??;
    let arm = format!("enzuiassistant arm {generation} {}", portal.handle());
    if let Err(error) = compositor_command(compositor, &arm, "compositor registration").await {
        let _ = compositor_check(compositor, "disarm", generation).await;
        return Err(error);
    }
    let result = tokio::select! {
        biased;
        result = cancel.revoked() => result,
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
) -> Attempt<()> {
    let (mut harness, peer) =
        tokio::time::timeout(IO_TIMEOUT, attach_harness(session_id, generation))
            .await
            .map_err(|_| ("harness attachment", "connection or handshake timed out"))??;
    let bus = zbus::fdo::DBusProxy::new(&portal.connection)
        .await
        .map_err(|_| ("portal backend", "could not create bus proxy"))?;
    let mut owners = bus
        .receive_name_owner_changed_with_args(&[(0, BACKEND)])
        .await
        .map_err(|_| ("portal backend", "could not watch backend owner"))?;
    let session = portal.session().await?;
    let mut closed = session
        .receive_signal("Closed")
        .await
        .map_err(|_| ("portal session", "could not subscribe to Closed"))?;
    tokio::select! {
        biased;
        _ = harness.read_u8() => return Err(("harness attachment", "harness closed during consent")),
        result = watch_compositor(compositor, generation, "pending") => return result,
        result = portal.start() => result?,
    }
    let pending = async {
        let stage = "portal backend";
        let backend = zbus::names::BusName::try_from(BACKEND)
            .map_err(|_| (stage, "invalid backend bus name"))?;
        if !bus
            .name_has_owner(backend)
            .await
            .map_err(|_| (stage, "could not query backend owner"))?
        {
            return Err((stage, "enzUI RemoteDesktop backend has no owner"));
        }
        let (indicator, shown) = Indicator::start(cancel.clone());
        tokio::time::timeout(IO_TIMEOUT, shown)
            .await
            .map_err(|_| {
                (
                    "indicator mapping",
                    "timed out waiting for a frame on every output",
                )
            })?
            .map_err(|_| {
                (
                    "indicator mapping",
                    "indicator stopped before its first frame",
                )
            })?;
        compositor_check(compositor, "visible", generation).await?;
        let fd = portal.connect_eis().await?;
        let mut eis =
            Eis::new(fd.into()).map_err(|_| ("ConnectToEIS", "could not initialize libei"))?;
        tokio::time::timeout(IO_TIMEOUT, async {
            while !eis.ready() {
                eis.dispatch()
                    .await
                    .map_err(|_| ("ConnectToEIS", "device setup failed"))?;
            }
            Ok::<(), Failure>(())
        })
        .await
        .map_err(|_| ("ConnectToEIS", "pointer or keyboard did not become ready"))??;
        compositor_check(compositor, "scoped", generation).await?;
        Ok((indicator, eis))
    };
    let (_indicator, mut eis) = tokio::select! {
        biased;
        _ = closed.next() => return Err(("portal session", "closed during setup")),
        _ = harness.read_u8() => return Err(("harness attachment", "harness closed during setup")),
        result = watch_compositor(compositor, generation, "pending") => return result,
        result = pending => result?,
    };
    let activation = "harness activation";
    tokio::time::timeout(IO_TIMEOUT, async {
        write_frame(
            harness.get_mut(),
            &json!({"generation":generation,"activate":true}),
        )
        .await
        .map_err(|_| (activation, "cannot send activation"))?;
        let reply = read_frame(&mut harness)
            .await
            .map_err(|_| (activation, "invalid or incomplete acknowledgement"))?;
        if reply["ok"] != true || reply["generation"] != generation || !peer.alive() {
            return Err((activation, "harness refused the lease"));
        }
        Ok(())
    })
    .await
    .map_err(|_| (activation, "acknowledgement timed out"))??;
    let backend_lost = async {
        while let Some(change) = owners.next().await {
            let change = change
                .args()
                .map_err(|_| ("portal backend", "invalid owner change"))?;
            if change.old_owner().as_ref().is_some() {
                break;
            }
        }
        Err((
            "portal backend",
            "backend owner disconnected or was replaced",
        ))
    };
    status
        .send(())
        .map_err(|()| ("client exchange", "client stopped waiting for activation"))?;
    tokio::select! {
        biased;
        _ = closed.next() => Err(("portal session", "closed while control was active")),
        result = backend_lost => result,
        result = watch_compositor(compositor, generation, "scoped") => result,
        result = async {
            let stage = "active input";
            loop {
                let mut request = {
                    let frame = read_frame(&mut harness);
                    tokio::pin!(frame);
                    loop {
                        tokio::select! {
                            biased;
                            result = eis.dispatch() => result.map_err(|_| (stage, "EIS connection or device lost"))?,
                            result = &mut frame => break result.map_err(|_| (stage, "harness disconnected or sent an invalid frame"))?,
                        }
                    }
                };
                if request["generation"] != generation || !peer.alive() || !cancel.allowed() {
                    return Err((stage, "lease or harness is no longer valid"));
                }
                compositor_check(compositor, "visible", generation).await?;
                let arguments = request["arguments"].take();
                let name = request["tool"].as_str().ok_or((stage, "missing input action"))?;
                let ok = eis.perform(name, arguments, || cancel.allowed())
                    .map_err(|_| (stage, "EIS rejected an input action"))?;
                tokio::time::timeout(IO_TIMEOUT,
                    write_frame(harness.get_mut(), &json!({"generation":generation,"ok":ok})))
                    .await.map_err(|_| (stage, "action reply timed out"))?
                    .map_err(|_| (stage, "cannot send action reply"))?;
            }
        } => result,
    }
}

async fn attach_harness(session: &str, generation: &str) -> Attempt<(BufReader<UnixStream>, Peer)> {
    let stage = "harness attachment";
    let path = std::env::var_os("ENZUI_AGENT_SOCKET")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(crate::agent_harness::socket_path)
        .ok_or((stage, "missing runtime directory"))?;
    let socket = UnixStream::connect(&path)
        .await
        .map_err(|_| (stage, "cannot connect to the agent harness socket"))?;
    let installed = Path::new("/usr/bin/enzimos-agent-harness");
    let preview = executable("enzimos-agent-harness")
        .map_err(|_| (stage, "cannot locate expected harness executable"))?;
    let expected = if crate::desktop_session::mode().nested() {
        preview.as_path()
    } else {
        installed
    };
    let peer = Peer::new(&socket, &[installed, expected])
        .map_err(|reason| ("harness peer check", reason))?;
    let mut socket = BufReader::new(socket);
    write_frame(
        socket.get_mut(),
        &json!({
            "schema_version": 1, "method": "session.desktop_control",
            "session_id": session, "generation": generation,
        }),
    )
    .await
    .map_err(|_| (stage, "cannot send attachment request"))?;
    let reply = read_frame(&mut socket)
        .await
        .map_err(|_| (stage, "invalid or incomplete reply"))?;
    if reply["ok"] != true || reply["generation"] != generation || !peer.alive() {
        return Err((stage, "harness refused the conversation or lease"));
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
        assert_eq!(revoked.revoked().await, Ok(()));
        assert!(!revoked.allowed());
        assert!(next.allowed());
    }
}
