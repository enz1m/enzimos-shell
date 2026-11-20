//! Inhibitor-aware idle timers; stopping restores only outputs this worker powered off.
//! Previews require explicit opt-in.

use enzui_shell::i18n::tr;
use std::{
    collections::BTreeMap,
    os::{fd::AsFd, unix::net::UnixStream},
    time::Duration,
};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
    protocol::{wl_callback, wl_output, wl_registry, wl_seat},
};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};
use wayland_protocols_wlr::output_power_management::v1::client::{
    zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1,
    zwlr_output_power_v1::{self, ZwlrOutputPowerV1},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Config {
    pub blank: Option<Duration>,
    pub lock: Option<Duration>,
    pub suspend: Option<Duration>,
}
impl Config {
    pub fn from_preferences(
        p: &crate::session_preferences::SessionPreferences,
        on_battery: bool,
    ) -> Self {
        let minutes = |m: u16| (m > 0).then(|| Duration::from_secs(u64::from(m) * 60));
        Self {
            blank: minutes(p.blank_minutes),
            lock: p
                .lock_after_minutes()
                .map(|m| Duration::from_secs(u64::from(m) * 60)),
            suspend: minutes(if on_battery {
                p.suspend_battery_minutes
            } else {
                p.suspend_ac_minutes
            }),
        }
    }
}

#[derive(Default)]
struct SessionWake {
    inactive: bool,
}

impl SessionWake {
    fn observe(&mut self, state: crate::session_activity::State) -> bool {
        use crate::session_activity::State;
        match state {
            State::Inactive => self.inactive = true,
            State::Active | State::Locked if self.inactive => {
                self.inactive = false;
                return true;
            }
            _ => {}
        }
        false
    }
}

#[derive(Clone, Debug)]
pub struct Notice {
    pub event: Event,
    generation: u64,
    epoch: std::sync::Arc<std::sync::atomic::AtomicU64>,
}
impl Notice {
    pub fn current(&self) -> bool {
        !matches!(self.event, Event::Lock | Event::Suspend)
            || self.generation == self.epoch.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[derive(Default)]
struct Epoch(std::sync::Arc<std::sync::atomic::AtomicU64>);
impl Epoch {
    fn invalidate(&self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    fn notice(&self, event: Event) -> Notice {
        Notice {
            event,
            generation: self.0.load(std::sync::atomic::Ordering::SeqCst),
            epoch: std::sync::Arc::clone(&self.0),
        }
    }
}
impl Drop for Epoch {
    fn drop(&mut self) {
        self.invalidate();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Lock,
    Suspend,
    Awake,
    Unavailable(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Timer {
    Blank,
    Lock,
    Suspend,
}

struct Output {
    output: wl_output::WlOutput,
    control: Option<ZwlrOutputPowerV1>,
    requested_off: bool,
}
struct State {
    seat: Option<(u32, wl_seat::WlSeat)>,
    notifier: Option<(u32, ExtIdleNotifierV1)>,
    power: Option<(u32, ZwlrOutputPowerManagerV1)>,
    outputs: BTreeMap<u32, Output>,
    notifications: Vec<ExtIdleNotificationV1>,
    blanked: bool,
    config: Config,
    initialized: bool,
    registry_ready: bool,
    events: tokio::sync::mpsc::Sender<Event>,
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
            } => {
                match interface.as_str() {
                    "wl_seat" if state.seat.is_none() => {
                        state.seat = Some((name, registry.bind(name, version.min(5), qh, ())));
                    }
                    "ext_idle_notifier_v1" if state.notifier.is_none() => {
                        state.notifier = Some((name, registry.bind(name, 1, qh, ())));
                    }
                    "zwlr_output_power_manager_v1" if state.power.is_none() => {
                        state.power = Some((name, registry.bind(name, 1, qh, ())));
                    }
                    "wl_output" => {
                        state.outputs.insert(
                            name,
                            Output {
                                output: registry.bind(name, version.min(4), qh, ()),
                                control: None,
                                requested_off: false,
                            },
                        );
                    }
                    _ => {}
                }
                if state.initialized {
                    state.bind_controls(qh);
                    if state.notifications.is_empty() {
                        state.arm(qh);
                    }
                }
            }
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(output) = state.outputs.remove(&name) {
                    if let Some(control) = output.control {
                        control.destroy();
                    }
                    if output.output.version() >= 3 {
                        output.output.release();
                    }
                }
                if state.seat.as_ref().is_some_and(|(id, _)| *id == name)
                    || state.notifier.as_ref().is_some_and(|(id, _)| *id == name)
                {
                    state.disarm();
                    if state.seat.as_ref().is_some_and(|(id, _)| *id == name) {
                        state.seat = None;
                    }
                    if state.notifier.as_ref().is_some_and(|(id, _)| *id == name) {
                        state.notifier = None;
                    }
                    state.set_power(true);
                    let _ = state.events.blocking_send(Event::Unavailable(
                        tr("shell-the-compositor-s-idle-seat-or-notifier-was-removed").into(),
                    ));
                }
                if state.power.as_ref().is_some_and(|(id, _)| *id == name) {
                    state.set_power(true);
                    for output in state.outputs.values_mut() {
                        if let Some(control) = output.control.take() {
                            control.destroy();
                        }
                    }
                    state.power = None;
                }
            }
            _ => {}
        }
    }
}
impl Dispatch<ExtIdleNotificationV1, Timer> for State {
    fn event(
        state: &mut Self,
        notification: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        timer: &Timer,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if !state.notifications.contains(notification) {
            return;
        }
        match (event, timer) {
            (ext_idle_notification_v1::Event::Idled, Timer::Blank) => state.set_power(false),
            (ext_idle_notification_v1::Event::Idled, Timer::Lock) => {
                let _ = state.events.blocking_send(Event::Lock);
            }
            (ext_idle_notification_v1::Event::Idled, Timer::Suspend) => {
                let _ = state.events.blocking_send(Event::Suspend);
            }
            (ext_idle_notification_v1::Event::Resumed, Timer::Blank) if state.blanked => {
                state.set_power(true);
                let _ = state.events.blocking_send(Event::Awake);
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.registry_ready = true;
    }
}
impl Dispatch<ZwlrOutputPowerV1, u32> for State {
    fn event(
        state: &mut Self,
        control: &ZwlrOutputPowerV1,
        event: zwlr_output_power_v1::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, zwlr_output_power_v1::Event::Failed)
            && let Some(output) = state
                .outputs
                .get_mut(name)
                .filter(|output| output.control.as_ref() == Some(control))
        {
            output.control = None;
            output.requested_off = false;
            let _ = state.events.blocking_send(Event::Unavailable(
                tr("shell-output-power-control-is-unavailable").into(),
            ));
        }
    }
}
wayland_client::delegate_noop!(State: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(State: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(State: ExtIdleNotifierV1);
wayland_client::delegate_noop!(State: ZwlrOutputPowerManagerV1);

impl State {
    fn bind_controls(&mut self, qh: &QueueHandle<Self>) {
        let Some((_, power)) = &self.power else {
            return;
        };
        for (name, output) in &mut self.outputs {
            if output.control.is_none() {
                let control = power.get_output_power(&output.output, qh, *name);
                if self.blanked {
                    control.set_mode(zwlr_output_power_v1::Mode::Off);
                    output.requested_off = true;
                }
                output.control = Some(control);
            }
        }
    }
    fn set_power(&mut self, on: bool) {
        self.blanked = !on;
        for output in self.outputs.values_mut() {
            if let Some(control) = &output.control
                && (!on || output.requested_off)
            {
                control.set_mode(if on {
                    zwlr_output_power_v1::Mode::On
                } else {
                    zwlr_output_power_v1::Mode::Off
                });
                output.requested_off = !on;
            }
        }
    }
    fn disarm(&mut self) {
        for notification in self.notifications.drain(..) {
            notification.destroy();
        }
    }
    fn arm(&mut self, qh: &QueueHandle<Self>) {
        self.disarm();
        let (Some((_, notifier)), Some((_, seat))) = (self.notifier.as_ref(), self.seat.as_ref())
        else {
            return;
        };
        for (timer, after) in [
            (Timer::Blank, self.config.blank),
            (Timer::Lock, self.config.lock),
            (Timer::Suspend, self.config.suspend),
        ] {
            if let Some(after) = after {
                self.notifications.push(notifier.get_idle_notification(
                    after.as_millis().min(u128::from(u32::MAX)) as u32,
                    seat,
                    qh,
                    timer,
                ));
            }
        }
    }
}

fn wait(display: &impl AsFd, stop: &impl AsFd) -> Result<bool, String> {
    let mut fds = [
        rustix::event::PollFd::new(display, rustix::event::PollFlags::IN),
        rustix::event::PollFd::new(stop, rustix::event::PollFlags::IN),
    ];
    loop {
        match rustix::event::poll(&mut fds, None) {
            Err(rustix::io::Errno::INTR) => continue,
            Err(_) => return Err(tr("shell-idle-event-wait-failed").into()),
            Ok(_) => break,
        }
    }
    if !fds[1].revents().is_empty() {
        return Ok(false);
    }
    if fds[0].revents().intersects(
        rustix::event::PollFlags::ERR
            | rustix::event::PollFlags::HUP
            | rustix::event::PollFlags::NVAL,
    ) {
        return Err(tr("shell-the-display-server-stopped-responding").into());
    }
    Ok(true)
}

/// Runs on a dedicated blocking thread until the stop socket closes; restores requested-off outputs on exit.
pub fn run(
    config: Config,
    stop: UnixStream,
    events: tokio::sync::mpsc::Sender<Event>,
) -> Result<(), String> {
    // Configuration replacements serialize output restoration before the next worker can blank.
    static WORKER: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _worker = WORKER
        .lock()
        .map_err(|_| tr("shell-idle-worker-serialization-failed"))?;
    let mut cancelled = [rustix::event::PollFd::new(
        &stop,
        rustix::event::PollFlags::IN,
    )];
    rustix::event::poll(
        &mut cancelled,
        Some(&rustix::event::Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        }),
    )
    .map_err(|_| tr("shell-idle-shutdown-check-failed"))?;
    if !cancelled[0].revents().is_empty() {
        return Ok(());
    }
    let connection =
        Connection::connect_to_env().map_err(|_| tr("shell-no-display-server-connection"))?;
    run_connection(config, stop, events, connection)
}

fn run_connection(
    config: Config,
    stop: UnixStream,
    events: tokio::sync::mpsc::Sender<Event>,
    connection: Connection,
) -> Result<(), String> {
    let mut queue: EventQueue<State> = connection.new_event_queue();
    let qh = queue.handle();
    connection.display().get_registry(&qh, ());
    let mut state = State {
        seat: None,
        notifier: None,
        power: None,
        outputs: BTreeMap::new(),
        notifications: Vec::new(),
        blanked: false,
        config,
        initialized: false,
        registry_ready: false,
        events,
    };
    connection.display().sync(&qh, ());
    while !state.registry_ready {
        queue
            .dispatch_pending(&mut state)
            .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
        connection
            .flush()
            .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
        if state.registry_ready {
            break;
        }
        if let Some(guard) = connection.prepare_read() {
            if !wait(&guard.connection_fd(), &stop)? {
                return Ok(());
            }
            guard
                .read()
                .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
        }
    }
    if state.notifier.is_none() || state.seat.is_none() {
        return Err(tr("shell-the-compositor-does-not-report-seat-idleness").into());
    }
    state.initialized = true;
    state.bind_controls(&qh);
    state.arm(&qh);
    let result = (|| {
        loop {
            queue
                .dispatch_pending(&mut state)
                .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
            connection
                .flush()
                .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
            if let Some(guard) = connection.prepare_read() {
                if !wait(&guard.connection_fd(), &stop)? {
                    drop(guard);
                    return Ok(());
                }
                guard
                    .read()
                    .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
            }
        }
    })();
    let restoring = state
        .outputs
        .values()
        .any(|output| output.requested_off && output.control.is_some());
    state.set_power(true);
    state.disarm();
    let restored = if restoring {
        // A flush followed by disconnect can discard the restore; wait for server acknowledgement.
        acknowledge_restore(&mut queue, &mut state, &connection, Duration::from_secs(1))
    } else {
        connection
            .flush()
            .map_err(|_| tr("shell-output-power-restoration-could-not-be-sent").into())
    };
    result.and(restored)
}

fn acknowledge_restore(
    queue: &mut EventQueue<State>,
    state: &mut State,
    connection: &Connection,
    timeout: Duration,
) -> Result<(), String> {
    state.registry_ready = false;
    connection.display().sync(&queue.handle(), ());
    let deadline = std::time::Instant::now() + timeout;
    loop {
        queue
            .dispatch_pending(state)
            .map_err(|_| tr("shell-output-power-restoration-could-not-be-acknowledged"))?;
        connection
            .flush()
            .map_err(|_| tr("shell-output-power-restoration-could-not-be-sent"))?;
        if state.registry_ready {
            return Ok(());
        }
        let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
            return Err(tr("shell-output-power-restoration-acknowledgement-timed-out").into());
        };
        if let Some(guard) = connection.prepare_read() {
            let fd = guard.connection_fd();
            let mut fds = [rustix::event::PollFd::new(
                &fd,
                rustix::event::PollFlags::IN,
            )];
            let timeout = rustix::event::Timespec {
                tv_sec: remaining.as_secs() as i64,
                tv_nsec: i64::from(remaining.subsec_nanos()),
            };
            match rustix::event::poll(&mut fds, Some(&timeout)) {
                Err(rustix::io::Errno::INTR) => continue,
                Err(_) => {
                    return Err(tr("shell-output-power-restoration-wait-failed").into());
                }
                Ok(0) => {
                    return Err(
                        tr("shell-output-power-restoration-acknowledgement-timed-out").into(),
                    );
                }
                Ok(_) => {}
            }
            guard
                .read()
                .map_err(|_| tr("shell-output-power-restoration-could-not-be-acknowledged"))?;
        }
    }
}

/// Changing configuration stops the old worker, which restores its requested-off outputs on exit.
pub fn subscription(config: Config) -> iced::Subscription<Notice> {
    if config == Config::default() {
        iced::Subscription::none()
    } else {
        iced::Subscription::run_with(config, stream)
    }
}

fn worker(config: Config) -> Result<(UnixStream, tokio::sync::mpsc::Receiver<Event>), String> {
    let (stop, keepalive) =
        UnixStream::pair().map_err(|_| tr("shell-idle-shutdown-channel-is-unavailable"))?;
    let (events, receiver) = tokio::sync::mpsc::channel(4);
    std::thread::spawn(move || {
        if let Err(error) = run(config, stop, events.clone()) {
            let _ = events.blocking_send(Event::Unavailable(error));
        }
    });
    Ok((keepalive, receiver))
}

struct Controller(tokio::task::JoinHandle<()>);
impl Drop for Controller {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn control(
    config: Config,
    output: tokio::sync::mpsc::Sender<Notice>,
    activity: impl iced::futures::Stream<Item = crate::session_activity::State>,
    start: impl Fn(Config) -> Result<(UnixStream, tokio::sync::mpsc::Receiver<Event>), String>,
) {
    use iced::futures::StreamExt;
    let epoch = Epoch::default();
    let (mut keepalive, mut receiver) = match start(config) {
        Ok(worker) => worker,
        Err(error) => {
            let _ = output.send(epoch.notice(Event::Unavailable(error))).await;
            return;
        }
    };
    iced::futures::pin_mut!(activity);
    let mut wake = SessionWake::default();
    let mut pending: Option<Notice> = None;
    loop {
        tokio::select! {
            biased;
            Some(state) = activity.next() => {
                if wake.observe(state) {
                    // Output restoration must run even when the GUI cannot receive frame callbacks.
                    epoch.invalidate();
                    pending = None;
                    drop(keepalive);
                    match start(config) {
                        Ok(worker) => { (keepalive, receiver) = worker; }
                        Err(error) => { let _ = output.send(epoch.notice(Event::Unavailable(error))).await; return; }
                    }
                }
            }
            result = async { output.send(pending.as_ref().expect("select branch is enabled only with a pending event").clone()).await }, if pending.is_some() => {
                if result.is_err() { return; }
                pending = None;
            }
            event = receiver.recv(), if pending.is_none() => {
                let Some(event) = event else { return; };
                pending = Some(epoch.notice(event));
            }
        }
    }
}

fn stream(config: &Config) -> impl iced::futures::Stream<Item = Notice> + use<> {
    use iced::futures::{SinkExt, StreamExt};
    let config = *config;
    iced::stream::channel(4, async move |mut output| {
        let activity = if crate::desktop_session::mode() == crate::desktop_session::Mode::Installed
            && !crate::desktop_session::remote()
        {
            crate::session_activity::stream().boxed()
        } else {
            iced::futures::stream::pending().boxed()
        };
        let (notices, mut receiver) = tokio::sync::mpsc::channel(4);
        // The consumer can stop polling while DPMS is off; the owned task must still restore power.
        let _controller = Controller(tokio::spawn(control(config, notices, activity, worker)));
        while let Some(notice) = receiver.recv().await {
            if output.send(notice).await.is_err() {
                return;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn idle_behaviour() {
        // Idle behaviour.
        {
            // Only a verified return restarts the worker even when still locked.
            {
                use crate::session_activity::State;
                let mut wake = SessionWake::default();
                for state in [State::Unknown, State::Active, State::Locked, State::Active] {
                    assert!(!wake.observe(state));
                }
                assert!(!wake.observe(State::Inactive));
                assert!(!wake.observe(State::Unknown));
                assert!(wake.observe(State::Locked));
                for state in [State::Locked, State::Active, State::Unknown, State::Active] {
                    assert!(!wake.observe(state));
                }
                assert!(!wake.observe(State::Inactive));
                assert!(wake.observe(State::Active));
            }
            // Restoration has a deadline when the compositor does not acknowledge.
            {
                let (display, _server) = UnixStream::pair().unwrap();
                let connection = Connection::from_socket(display).unwrap();
                let mut queue = connection.new_event_queue();
                let mut state = restore_state();
                let before = std::time::Instant::now();
                assert!(
                    acknowledge_restore(
                        &mut queue,
                        &mut state,
                        &connection,
                        Duration::from_millis(20)
                    )
                    .unwrap_err()
                    .contains("timed out")
                );
                assert!(before.elapsed() < Duration::from_secs(1));
            }
        }

        use crate::session_activity::State;
        use iced::futures::{SinkExt, channel::mpsc};
        use std::{
            io::Read,
            sync::{
                Arc, Mutex,
                atomic::{AtomicUsize, Ordering},
            },
        };
        let starts = Arc::new(AtomicUsize::new(0));
        let peers = Arc::new(Mutex::new(Vec::new()));
        let senders = Arc::new(Mutex::new(Vec::new()));
        let (mut activity, states) = mpsc::channel(4);
        let (output, mut gui) = tokio::sync::mpsc::channel(4);
        let start = {
            let starts = std::sync::Arc::clone(&starts);
            let peers = std::sync::Arc::clone(&peers);
            move |_| {
                let (stop, keepalive) = UnixStream::pair().unwrap();
                peers.lock().unwrap().push(stop);
                let (events, receiver) = tokio::sync::mpsc::channel(8);
                for _ in 0..8 {
                    events.try_send(Event::Lock).unwrap();
                }
                senders.lock().unwrap().push(events);
                starts.fetch_add(1, Ordering::SeqCst);
                Ok((keepalive, receiver))
            }
        };
        let controller = Controller(tokio::spawn(control(
            Config::default(),
            output,
            states,
            start,
        )));
        tokio::time::timeout(Duration::from_secs(3), async {
            while gui.len() != 4 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // Do not drain the full GUI queue until the inactive/return pair has restarted the worker.
        activity.send(State::Inactive).await.unwrap();
        activity.send(State::Active).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while starts.load(Ordering::SeqCst) != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!gui.recv().await.unwrap().current());
        drop(controller);
        tokio::task::yield_now().await;
        let peers = &mut *peers.lock().unwrap();
        assert_eq!(peers.len(), 2);
        for peer in peers {
            peer.set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            assert_eq!(peer.read(&mut [0]).unwrap(), 0);
        }
    }
    fn restore_state() -> State {
        let (events, _) = tokio::sync::mpsc::channel(4);
        State {
            seat: None,
            notifier: None,
            power: None,
            outputs: BTreeMap::new(),
            notifications: Vec::new(),
            blanked: false,
            config: Config::default(),
            initialized: false,
            registry_ready: false,
            events,
        }
    }
}
