use crate::clipboard::session;
use enzui_shell::i18n::tr;
use iced::futures::{SinkExt, Stream};
use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};
use std::{fs, os::unix::fs::MetadataExt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receiver {
    pid: u32,
    start: u64,
    device: u64,
    inode: u64,
}
impl Receiver {
    fn capture(pid: u32) -> Result<Self, String> {
        let executable = fs::metadata(format!("/proc/{pid}/exe"))
            .map_err(|_| tr("shell-shell-receiver-is-unavailable"))?;
        Ok(Self {
            pid,
            start: session::start(pid).map_err(|_| tr("shell-shell-receiver-is-unavailable"))?,
            device: executable.dev(),
            inode: executable.ino(),
        })
    }
    pub fn parse(values: &[String]) -> Result<Self, String> {
        if values.len() != 4
            || values.iter().any(|value| {
                value.is_empty()
                    || value.len() > 20
                    || !value.bytes().all(|byte| byte.is_ascii_digit())
            })
        {
            return Err(tr("shell-invalid-shell-receiver-identity").into());
        }
        let numbers = values
            .iter()
            .map(|value| value.parse::<u64>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| tr("shell-invalid-shell-receiver-identity"))?;
        let pid = u32::try_from(numbers[0])
            .ok()
            .filter(|pid| *pid > 0 && *pid <= i32::MAX as u32)
            .ok_or_else(|| tr("shell-invalid-shell-receiver-identity"))?;
        Ok(Self {
            pid,
            start: numbers[1],
            device: numbers[2],
            inode: numbers[3],
        })
    }
    pub fn arguments(&self) -> String {
        format!("{} {} {} {}", self.pid, self.start, self.device, self.inode)
    }
    fn signal(&self, signal: Signal) -> Result<(), String> {
        let pid = Pid::from_raw(self.pid as i32)
            .ok_or_else(|| tr("shell-invalid-shell-receiver-process"))?;
        let fd =
            pidfd_open(pid, PidfdFlags::empty()).map_err(|_| tr("shell-shell-receiver-exited"))?;
        if Self::capture(self.pid)? != *self {
            return Err(tr("shell-shell-receiver-identity-changed").into());
        }
        pidfd_send_signal(&fd, signal).map_err(|_| tr("shell-shell-receiver-exited").into())
    }
}
fn scoped_target() -> Result<session::Target, String> {
    let target = session::Target::from_environment()
        .map_err(|_| tr("shell-shortcut-requires-a-managed-desktop"))?;
    drop(
        target
            .connect()
            .map_err(|_| tr("shell-shortcut-desktop-identity-changed"))?,
    );
    Ok(target)
}
pub fn open_notifications(receiver: Receiver) -> Result<(), String> {
    open(receiver, Signal::USR1)
}
pub fn open_search(receiver: Receiver) -> Result<(), String> {
    open(receiver, Signal::USR2)
}
fn open(receiver: Receiver, signal: Signal) -> Result<(), String> {
    let target = scoped_target()?;
    let config: serde_json::Value = serde_json::from_slice(
        &session::private_bytes(&target.runtime().join("launch.json"), 32768)
            .map_err(|_| tr("shell-shortcut-scope-is-unavailable"))?,
    )
    .map_err(|_| tr("shell-shortcut-scope-is-invalid"))?;
    let unit = config["unit"]
        .as_str()
        .ok_or_else(|| tr("shell-shortcut-scope-is-invalid"))?;
    session::membership(std::process::id(), unit)
        .map_err(|_| tr("shell-shortcut-caller-scope-changed"))?;
    session::membership(receiver.pid, unit)
        .map_err(|_| tr("shell-shell-receiver-is-outside-the-managed-supervisor"))?;
    let pid = session::private_text(&target.runtime().join("shell.pid"), 32)
        .map_err(|_| tr("shell-shell-receiver-receipt-is-unavailable"))?;
    let start = session::private_text(&target.runtime().join("shell.pid.start"), 32)
        .map_err(|_| tr("shell-shell-receiver-receipt-is-unavailable"))?;
    if pid.parse::<u32>().ok() != Some(receiver.pid)
        || start.parse::<u64>().ok() != Some(receiver.start)
    {
        return Err(tr("shell-shell-receiver-receipt-changed").into());
    }
    receiver.signal(signal)
}
#[derive(Clone, Debug)]
pub enum Event {
    ReceiverInstalled(Receiver),
    OpenNotifications,
    OpenSearch,
    Unavailable(String),
}
pub fn subscription() -> iced::Subscription<Event> {
    iced::Subscription::run(events)
}
fn events() -> impl Stream<Item = Event> {
    iced::stream::channel(4, async |mut output| {
        let identity = tokio::task::spawn_blocking(|| {
            scoped_target()?;
            Receiver::capture(std::process::id())
        })
        .await
        .map_err(|_| tr("shell-shortcut-scope-reader-stopped").to_owned())
        .and_then(|result| result);
        let result = async {
            let identity = identity?;
            let mut signal =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined1())
                    .map_err(|_| tr("shell-shortcut-receiver-is-unavailable"))?;
            let mut search =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined2())
                    .map_err(|_| tr("shell-search-shortcut-receiver-is-unavailable"))?;
            output
                .send(Event::ReceiverInstalled(identity))
                .await
                .map_err(|_| tr("shell-shortcut-frontend-closed"))?;
            loop {
                let event = tokio::select! {
                    event = signal.recv() => event.map(|()| Event::OpenNotifications),
                    event = search.recv() => event.map(|()| Event::OpenSearch),
                };
                let Some(event) = event else {
                    break;
                };
                output
                    .send(event)
                    .await
                    .map_err(|_| tr("shell-shortcut-frontend-closed"))?;
            }
            Err::<(), String>(tr("shell-shortcut-receiver-stopped").into())
        }
        .await;
        if let Err(error) = result {
            let _ = output.send(Event::Unavailable(error)).await;
        }
        std::future::pending::<()>().await;
    })
}
