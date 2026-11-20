use super::{
    content::{Content, Mime, SELF_MIME, choose},
    session::Target,
};
use enzui_shell::i18n::tr;
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    fs::{FileType, OFlags, fcntl_getfl, fcntl_setfl, fstat},
    pipe::{PipeFlags, pipe_with},
};
use smithay_client_toolkit::reexports::client::{
    Connection, Dispatch, QueueHandle, delegate_noop, event_created_child,
    protocol::{wl_callback, wl_registry, wl_seat},
};
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Write},
    os::fd::{AsFd, OwnedFd},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1 as Device},
    ext_data_control_manager_v1::ExtDataControlManagerV1 as Manager,
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1 as Offer},
    ext_data_control_source_v1::{self, ExtDataControlSourceV1 as Source},
};

pub enum Command {
    Collect(bool),
    Restore {
        request: u64,
        content: Arc<Content>,
        authority: Arc<AtomicBool>,
    },
}
#[derive(Clone)]
pub enum Event {
    Ready,
    Copied {
        content: Arc<Content>,
        started: Instant,
    },
    Restored {
        request: u64,
        result: Result<(), String>,
    },
    Rejected {
        error: String,
        started: Instant,
    },
}
#[derive(Default)]
struct Metadata {
    mimes: Vec<String>,
    invalid: bool,
}
struct Incoming {
    pipe: File,
    bytes: Vec<u8>,
    mime: Mime,
    started: Instant,
}
struct Outgoing {
    pipe: File,
    content: Arc<Content>,
    written: usize,
    started: Instant,
}
struct Selection {
    request: u64,
    source: Source,
    content: Arc<Content>,
    authority: Arc<AtomicBool>,
}
enum Confirmation {
    Startup,
    Restore(u64),
}
struct State {
    manager: Option<Manager>,
    seat: Option<wl_seat::WlSeat>,
    globals: Vec<u32>,
    device: Option<Device>,
    initialized: bool,
    collecting: bool,
    offers: HashMap<Offer, Metadata>,
    current: Option<Offer>,
    incoming: Option<Incoming>,
    outgoing: Vec<Outgoing>,
    selection: Option<Selection>,
    confirmations: Vec<(u64, Instant)>,
    events: mpsc::Sender<Event>,
    error: Option<String>,
}
fn error(_: impl std::fmt::Display) -> String {
    tr("shell-clipboard-transport-is-unavailable").into()
}
fn nonblocking(fd: &impl AsFd) -> Result<(), String> {
    fcntl_setfl(fd, fcntl_getfl(fd).map_err(error)? | OFlags::NONBLOCK).map_err(error)
}
impl State {
    fn emit(&mut self, event: Event) {
        if self.events.try_send(event).is_err() {
            self.error = Some(tr("shell-clipboard-consumer-is-unavailable-or-busy").into());
        }
    }
    fn select(&mut self, offer: Option<Offer>) {
        self.incoming = None;
        if let Some(old) = self.current.take() {
            self.offers.remove(&old);
            old.destroy();
        }
        let Some(offer) = offer else {
            return;
        };
        self.current = Some(offer);
        self.receive_current();
    }
    fn collect(&mut self, enabled: bool) {
        if self.collecting != enabled {
            self.collecting = enabled;
            self.incoming = None;
            if enabled {
                self.receive_current();
            }
        }
    }
    fn receive_current(&mut self) {
        if !self.collecting {
            return;
        }
        let Some(offer) = self.current.as_ref() else {
            return;
        };
        let Some(meta) = self.offers.get(offer).filter(|m| !m.invalid) else {
            return;
        };
        let Some((mime, name)) = choose(&meta.mimes) else {
            return;
        };
        let result = (|| {
            let (read, write) = pipe_with(PipeFlags::CLOEXEC).map_err(error)?;
            nonblocking(&read)?;
            offer.receive(name, write.as_fd());
            Ok::<_, String>(Incoming {
                pipe: File::from(read),
                bytes: Vec::new(),
                mime,
                started: Instant::now(),
            })
        })();
        match result {
            Ok(incoming) => self.incoming = Some(incoming),
            Err(reason) => self.error = Some(reason),
        }
    }
    fn restore(
        &mut self,
        request: u64,
        content: Arc<Content>,
        authority: Arc<AtomicBool>,
        connection: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let result = (|| {
            if self.confirmations.len() >= 8 {
                return Err(tr("shell-clipboard-restore-queue-is-full").into());
            }
            if !authority.load(Ordering::Acquire) {
                return Err(tr("shell-clipboard-restore-authority-changed").into());
            }
            let manager = self
                .manager
                .as_ref()
                .ok_or_else(|| tr("shell-clipboard-is-not-ready"))?;
            let device = self
                .device
                .as_ref()
                .ok_or_else(|| tr("shell-clipboard-is-not-ready"))?;
            let source = manager.create_data_source(qh, ());
            for mime in content.mime.names() {
                source.offer((*mime).into());
            }
            source.offer(SELF_MIME.into());
            if !authority.load(Ordering::Acquire) {
                source.destroy();
                return Err(tr("shell-clipboard-restore-authority-changed").into());
            }
            device.set_selection(Some(&source));
            if let Some(old) = self.selection.take() {
                old.source.destroy();
            }
            self.outgoing.clear();
            self.selection = Some(Selection {
                request,
                source,
                content,
                authority,
            });
            self.confirmations.push((request, Instant::now()));
            connection
                .display()
                .sync(qh, Confirmation::Restore(request));
            Ok(())
        })();
        if let Err(reason) = result {
            self.emit(Event::Restored {
                request,
                result: Err(reason),
            });
        }
    }
    fn send(&mut self, source: &Source, mime: String, fd: OwnedFd) {
        let Some(selection) = self
            .selection
            .as_ref()
            .filter(|s| s.source == *source && s.authority.load(Ordering::Acquire))
        else {
            return;
        };
        if self.outgoing.len() >= 4 || !selection.content.mime.names().contains(&mime.as_str()) {
            return;
        }
        if fstat(&fd)
            .ok()
            .is_none_or(|stat| FileType::from_raw_mode(stat.st_mode) != FileType::Fifo)
            || nonblocking(&fd).is_err()
        {
            return;
        }
        self.outgoing.push(Outgoing {
            pipe: File::from(fd),
            content: std::sync::Arc::clone(&selection.content),
            written: 0,
            started: Instant::now(),
        });
    }
    fn transfer(&mut self) {
        if self
            .selection
            .as_ref()
            .is_some_and(|s| !s.authority.load(Ordering::Acquire))
        {
            if let Some(selection) = self.selection.take() {
                selection.source.destroy();
            }
            self.outgoing.clear();
        }
        if let Some(mut incoming) = self.incoming.take() {
            let mut complete = false;
            let mut failed = incoming.started.elapsed() >= Duration::from_secs(3);
            if !failed {
                let mut buffer = [0; 16384];
                for _ in 0..16 {
                    match incoming.pipe.read(&mut buffer) {
                        Ok(0) => {
                            complete = true;
                            break;
                        }
                        Ok(n) if incoming.bytes.len() + n <= incoming.mime.limit() => {
                            incoming.bytes.extend_from_slice(&buffer[..n]);
                        }
                        Ok(_) => {
                            failed = true;
                            break;
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => {
                            failed = true;
                            break;
                        }
                    }
                }
            }
            if complete {
                if !incoming.bytes.is_empty() {
                    match Content::decode(incoming.mime, incoming.bytes) {
                        Ok(content) => {
                            self.emit(Event::Copied {
                                content: Arc::new(content),
                                started: incoming.started,
                            });
                        }
                        Err(error) => {
                            self.emit(Event::Rejected {
                                error,
                                started: incoming.started,
                            });
                        }
                    }
                }
            } else if failed {
                self.emit(Event::Rejected {
                    error: tr("shell-clipboard-transfer-failed-or-exceeded-its-limits").into(),
                    started: incoming.started,
                });
            } else {
                self.incoming = Some(incoming);
            }
        }
        self.outgoing.retain_mut(|outgoing| {
            if outgoing.started.elapsed() >= Duration::from_secs(3) {
                return false;
            }
            for _ in 0..16 {
                let end = (outgoing.written + 16384).min(outgoing.content.bytes.len());
                match outgoing
                    .pipe
                    .write(&outgoing.content.bytes[outgoing.written..end])
                {
                    Ok(0) => return false,
                    Ok(n) => outgoing.written += n,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return true,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => return false,
                }
                if outgoing.written == outgoing.content.bytes.len() {
                    return false;
                }
            }
            true
        });
    }
}
impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == "ext_data_control_manager_v1" => {
                if state.manager.is_some() {
                    state.error = Some(tr("shell-ambiguous-clipboard-manager").into());
                    return;
                }
                state.manager = Some(registry.bind(name, version.min(1), qh, ()));
                state.globals.push(name);
            }
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == "wl_seat" => {
                if state.seat.is_some() {
                    state.error =
                        Some(tr("shell-clipboard-requires-exactly-one-desktop-seat").into());
                    return;
                }
                state.seat = Some(registry.bind(name, version.min(7), qh, ()));
                state.globals.push(name);
            }
            wl_registry::Event::GlobalRemove { name } if state.globals.contains(&name) => {
                state.error = Some(tr("shell-clipboard-seat-or-manager-was-removed").into());
            }
            _ => (),
        }
    }
}
impl Dispatch<wl_callback::WlCallback, Confirmation> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        confirmation: &Confirmation,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match confirmation {
            Confirmation::Startup => state.initialized = true,
            Confirmation::Restore(request) => {
                state.confirmations.retain(|(id, _)| id != request);
                let result =
                    if state.selection.as_ref().is_some_and(|s| {
                        s.request == *request && s.authority.load(Ordering::Acquire)
                    }) {
                        Ok(())
                    } else {
                        Err(tr("shell-clipboard-restore-was-superseded-or-revoked").into())
                    };
                state.emit(Event::Restored {
                    request: *request,
                    result,
                });
            }
        }
    }
}
impl Dispatch<Device, ()> for State {
    fn event(
        state: &mut Self,
        _: &Device,
        event: ext_data_control_device_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_device_v1::Event::DataOffer { id } => {
                if state.offers.len() >= 8 {
                    id.destroy();
                    state.error = Some(tr("shell-too-many-clipboard-offers").into());
                } else {
                    state.offers.insert(id, Metadata::default());
                }
            }
            ext_data_control_device_v1::Event::Selection { id } => state.select(id),
            ext_data_control_device_v1::Event::PrimarySelection { id: Some(offer) } => {
                state.offers.remove(&offer);
                offer.destroy();
            }
            ext_data_control_device_v1::Event::Finished => {
                state.error = Some(tr("shell-clipboard-device-was-removed").into());
            }
            _ => (),
        }
    }
    event_created_child!(State, Device, [0 => (Offer, ())]);
}
impl Dispatch<Offer, ()> for State {
    fn event(
        state: &mut Self,
        offer: &Offer,
        event: ext_data_control_offer_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event
            && let Some(meta) = state.offers.get_mut(offer)
        {
            if mime_type.len() > 256 || meta.mimes.len() >= 32 {
                meta.invalid = true;
            } else {
                meta.mimes.push(mime_type);
            }
        }
    }
}
impl Dispatch<Source, ()> for State {
    fn event(
        state: &mut Self,
        source: &Source,
        event: ext_data_control_source_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_source_v1::Event::Send { mime_type, fd } => {
                state.send(source, mime_type, fd);
            }
            ext_data_control_source_v1::Event::Cancelled => {
                if state
                    .selection
                    .as_ref()
                    .is_some_and(|s| s.source == *source)
                {
                    state.selection = None;
                    state.outgoing.clear();
                }
                source.destroy();
            }
            _ => (),
        }
    }
}
delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: Manager);

pub fn run(
    target: Target,
    mut commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<Event>,
    alive: Arc<AtomicBool>,
) -> Result<(), String> {
    let connection = Connection::from_socket(target.connect()?).map_err(error)?;
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    connection.display().get_registry(&qh, ());
    connection.display().sync(&qh, Confirmation::Startup);
    let mut state = State {
        manager: None,
        seat: None,
        globals: Vec::new(),
        device: None,
        initialized: false,
        collecting: false,
        offers: HashMap::new(),
        current: None,
        incoming: None,
        outgoing: Vec::new(),
        selection: None,
        confirmations: Vec::new(),
        events,
        error: None,
    };
    let started = Instant::now();
    while alive.load(Ordering::Acquire) {
        queue.dispatch_pending(&mut state).map_err(error)?;
        if let Some(error) = state.error.take() {
            return Err(error);
        }
        if state.initialized && state.device.is_none() {
            let manager = state
                .manager
                .as_ref()
                .ok_or_else(|| tr("shell-desktop-does-not-support-ext-data-control-v1"))?;
            let seat = state
                .seat
                .as_ref()
                .ok_or_else(|| tr("shell-desktop-has-no-clipboard-seat"))?;
            state.device = Some(manager.get_data_device(seat, &qh, ()));
            state.emit(Event::Ready);
        }
        if state
            .confirmations
            .iter()
            .any(|(_, at)| at.elapsed() >= Duration::from_secs(3))
        {
            return Err(tr("shell-clipboard-restore-confirmation-timed-out").into());
        }
        if !state.initialized && started.elapsed() >= Duration::from_secs(3) {
            return Err(tr("shell-clipboard-startup-timed-out").into());
        }
        for _ in 0..8 {
            match commands.try_recv() {
                Ok(Command::Collect(enabled)) => state.collect(enabled),
                Ok(Command::Restore {
                    request,
                    content,
                    authority,
                }) => state.restore(request, content, authority, &connection, &qh),
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => return Ok(()),
            }
        }
        state.transfer();
        let write_blocked = match connection.flush() {
            Ok(()) => false,
            Err(smithay_client_toolkit::reexports::client::backend::WaylandError::Io(e))
                if e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                true
            }
            Err(e) => return Err(error(e)),
        };
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let flags = PollFlags::IN
            | if write_blocked {
                PollFlags::OUT
            } else {
                PollFlags::empty()
            };
        let mut fds = vec![PollFd::new(&connection, flags)];
        if let Some(incoming) = &state.incoming {
            fds.push(PollFd::new(&incoming.pipe, PollFlags::IN));
        }
        for outgoing in &state.outgoing {
            fds.push(PollFd::new(&outgoing.pipe, PollFlags::OUT));
        }
        let result = poll(
            &mut fds,
            Some(&Timespec {
                tv_sec: 0,
                tv_nsec: 50_000_000,
            }),
        );
        match result {
            Err(rustix::io::Errno::INTR) => continue,
            Err(e) => return Err(error(e)),
            Ok(_) => (),
        }
        let ready = fds[0]
            .revents()
            .intersects(PollFlags::IN | PollFlags::ERR | PollFlags::HUP);
        drop(fds);
        if ready {
            guard.read().map_err(error)?;
        } else {
            drop(guard);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> (State, mpsc::Receiver<Event>) {
        let (events, receiver) = mpsc::channel(4);
        (
            State {
                manager: None,
                seat: None,
                globals: Vec::new(),
                device: None,
                initialized: true,
                collecting: false,
                offers: HashMap::new(),
                current: None,
                incoming: None,
                outgoing: Vec::new(),
                selection: None,
                confirmations: Vec::new(),
                events,
                error: None,
            },
            receiver,
        )
    }
    #[test]
    fn transport_behaviour() {
        // Incomplete timeout and oversize reads never publish partial content.
        {
            let (mut state, mut events) = state();
            let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
            nonblocking(&read).unwrap();
            state.incoming = Some(Incoming {
                pipe: File::from(read),
                bytes: b"partial".to_vec(),
                mime: Mime::Text,
                started: Instant::now() - Duration::from_secs(4),
            });
            state.transfer();
            assert!(state.incoming.is_none());
            assert!(matches!(events.try_recv(), Ok(Event::Rejected { .. })));
            assert!(events.try_recv().is_err());
            drop(write);
            let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
            nonblocking(&read).unwrap();
            File::from(write).write_all(b"x").unwrap();
            state.incoming = Some(Incoming {
                pipe: File::from(read),
                bytes: vec![b'a'; super::super::content::MAX_TEXT],
                mime: Mime::Text,
                started: Instant::now(),
            });
            state.transfer();
            assert!(state.incoming.is_none());
            assert!(matches!(events.try_recv(), Ok(Event::Rejected { .. })));
            assert!(events.try_recv().is_err());
        }
        // Invalid utf8 reports only a bounded diagnostic and empty selection is quiet.
        {
            for bytes in [vec![255], Vec::new()] {
                let invalid = !bytes.is_empty();
                let (mut state, mut events) = state();
                let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
                nonblocking(&read).unwrap();
                File::from(write).write_all(&bytes).unwrap();
                let started = Instant::now();
                state.incoming = Some(Incoming {
                    pipe: File::from(read),
                    bytes: Vec::new(),
                    mime: Mime::Text,
                    started,
                });
                state.transfer();
                if invalid {
                    let Event::Rejected {
                        error,
                        started: actual,
                    } = events.try_recv().unwrap()
                    else {
                        panic!()
                    };
                    assert_eq!(error, "Clipboard text is not UTF-8");
                    assert_eq!(actual, started);
                }
                assert!(events.try_recv().is_err());
                assert!(state.incoming.is_none());
            }
        }
        // Eof publishes complete content and backpressure is terminal.
        {
            let (mut state, mut events) = state();
            let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
            nonblocking(&read).unwrap();
            File::from(write).write_all(b"fixture").unwrap();
            state.incoming = Some(Incoming {
                pipe: File::from(read),
                bytes: Vec::new(),
                mime: Mime::Text,
                started: Instant::now(),
            });
            state.transfer();
            let Event::Copied { content, .. } = events.try_recv().unwrap() else {
                panic!("Expected clipboard event")
            };
            assert_eq!(&*content.bytes, b"fixture");
            for _ in 0..5 {
                state.emit(Event::Ready);
            }
            assert!(state.error.is_some());
        }
        // Stalled paste readers do not block or retain buffers forever.
        {
            let (mut state, _) = state();
            let (read, write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
            nonblocking(&write).unwrap();
            let content = Arc::new(
                Content::decode(Mime::Text, vec![b'a'; super::super::content::MAX_TEXT]).unwrap(),
            );
            state.outgoing.push(Outgoing {
                pipe: File::from(write),
                content,
                written: 0,
                started: Instant::now(),
            });
            state.transfer();
            assert_eq!(state.outgoing.len(), 1);
            assert!(state.outgoing[0].written > 0);
            state.outgoing[0].started = Instant::now() - Duration::from_secs(4);
            state.transfer();
            assert!(state.outgoing.is_empty());
            drop(read);
        }
    }
}
