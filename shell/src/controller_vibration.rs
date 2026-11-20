use enzui_shell::i18n::tr;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

pub const WORKER_ARGUMENT: &str = "--controller-vibration-worker";
pub const STOPPING: &str =
    "A previous vibration request is still stopping. Wait before trying again.";
static LEASE: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(1)));

pub fn has_pending_job() -> bool {
    LEASE.available_permits() == 0
}

const FAILED: &str = "The controller could not complete the vibration test. It may be disconnected, busy, or unavailable.";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    node: PathBuf,
    device: u64,
    inode: u64,
    rdev: u64,
}

fn event_node(path: &Path) -> bool {
    path.parent() == Some(Path::new("/dev/input"))
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("event"))
            .is_some_and(|number| {
                !number.is_empty()
                    && number.len() <= 5
                    && number.bytes().all(|b| b.is_ascii_digit())
            })
}

impl Target {
    pub fn capture(node: &Path) -> Result<Self, String> {
        if !event_node(node) {
            return Err(FAILED.into());
        }
        let metadata = fs::symlink_metadata(node).map_err(|_| FAILED)?;
        if !metadata.file_type().is_char_device() {
            return Err(FAILED.into());
        }
        Ok(Self {
            node: node.into(),
            device: metadata.dev(),
            inode: metadata.ino(),
            rdev: metadata.rdev(),
        })
    }

    fn matches(&self, metadata: &fs::Metadata) -> bool {
        metadata.file_type().is_char_device()
            && (metadata.dev(), metadata.ino(), metadata.rdev())
                == (self.device, self.inode, self.rdev)
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    Stopping,
    Finished(Result<(), String>),
}

struct Cancel(Option<tokio::sync::oneshot::Sender<()>>);
impl Drop for Cancel {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

pub fn run(target: Target) -> impl iced::futures::Stream<Item = Event> {
    run_with(std::env::current_exe(), target)
}

fn run_with(
    executable: std::io::Result<PathBuf>,
    target: Target,
) -> impl iced::futures::Stream<Item = Event> {
    // Admission belongs to the user action, before an executor can defer polling.
    let permit = LEASE.clone().try_acquire_owned();
    iced::stream::channel(
        2,
        async move |mut output: iced::futures::channel::mpsc::Sender<Event>| {
            use iced::futures::SinkExt;
            let Ok(permit) = permit else {
                let _ = output.send(Event::Finished(Err(STOPPING.into()))).await;
                return;
            };
            let child = executable.and_then(|executable| {
                tokio::process::Command::new(executable)
                    .arg(WORKER_ARGUMENT)
                    .arg(std::process::id().to_string())
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
            });
            let Ok(child) = child else {
                let _ = output.send(Event::Finished(Err(FAILED.into()))).await;
                return;
            };
            let (cancel, cancelled) = tokio::sync::oneshot::channel();
            let (sender, mut events) = tokio::sync::mpsc::channel(2);
            tokio::spawn(supervise(child, target, permit, cancelled, sender));
            let _cancel = Cancel(Some(cancel));
            while let Some(event) = events.recv().await {
                if output.send(event).await.is_err() {
                    break;
                }
            }
        },
    )
}

async fn supervise(
    mut child: tokio::process::Child,
    target: Target,
    _permit: tokio::sync::OwnedSemaphorePermit,
    mut cancelled: tokio::sync::oneshot::Receiver<()>,
    events: tokio::sync::mpsc::Sender<Event>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    enum Outcome {
        Exited(std::io::Result<std::process::ExitStatus>),
        Cancelled,
        Timeout,
    }
    let outcome = {
        let operation = async {
            let mut input = child
                .stdin
                .take()
                .ok_or_else(|| std::io::Error::other(tr("shell-missing-input")))?;
            let bytes = serde_json::to_vec(&target).map_err(std::io::Error::other)?;
            if bytes.len() > 1024 {
                return Err(std::io::Error::other(tr("shell-invalid-target")));
            }
            input.write_all(&bytes).await?;
            drop(input);
            child.wait().await
        };
        tokio::select! {
            biased;
            _ = &mut cancelled => Outcome::Cancelled,
            () = tokio::time::sleep(Duration::from_millis(3500)) => Outcome::Timeout,
            result = operation => Outcome::Exited(result),
        }
    };
    let status = match outcome {
        Outcome::Exited(Ok(status)) => Some(status),
        _ => {
            let _ = child.start_kill();
            let _ = events.send(Event::Stopping).await;
            // Kernel driver cleanup may defer exit even after SIGKILL. Keep the
            // child and its single-job lease until wait confirms actual reaping.
            let _ = child.wait().await;
            let _ = events.send(Event::Finished(Err(FAILED.into()))).await;
            None
        }
    };
    if let Some(status) = status {
        let mut bytes = Vec::new();
        if let Some(mut output) = child.stdout.take() {
            let _ = output.read_to_end(&mut bytes).await;
        }
        let result = match status.code() {
            Some(0) if bytes == b"ok\n" => Ok(()),
            Some(3) => Err(tr("shell-vibration-requires-an-unlocked-active-local-session").into()),
            Some(4) => Err(tr("shell-this-controller-does-not-support-vibration").into()),
            _ => Err(FAILED.into()),
        };
        let _ = events.send(Event::Finished(result)).await;
    }
}

#[allow(
    clippy::print_stdout,
    reason = "The parent requires the exact ok newline acknowledgement on stdout."
)]
pub fn worker(arguments: &[String]) -> i32 {
    use iced::futures::StreamExt;
    let [parent] = arguments else {
        return 2;
    };
    let Ok(parent) = parent.parse::<u32>() else {
        return 2;
    };
    let same_parent = || {
        rustix::process::getppid().is_some_and(|pid| pid.as_raw_nonzero().get() as u32 == parent)
    };
    if !same_parent()
        || rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::KILL))
            .is_err()
        || !same_parent()
    {
        return 2;
    }
    // Request exit if the driver stops responding; the parent retains ownership until reaped.
    if std::thread::Builder::new()
        .name("vibration-deadline".into())
        .spawn(|| {
            std::thread::sleep(Duration::from_secs(3));
            std::process::exit(124);
        })
        .is_err()
    {
        return 2;
    }
    let mut bytes = Vec::new();
    if std::io::stdin().take(1025).read_to_end(&mut bytes).is_err() || bytes.len() > 1024 {
        return 2;
    }
    let Ok(target) = serde_json::from_slice::<Target>(&bytes) else {
        return 2;
    };
    if !event_node(&target.node) {
        return 2;
    }
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return 2;
    };
    // D-Bus match-stream teardown queues work on this runtime too.
    let _entered = runtime.enter();
    let active = runtime.block_on(async {
        let input = crate::session_activity::stream();
        iced::futures::pin_mut!(input);
        tokio::time::timeout(Duration::from_secs(2), input.next())
            .await
            .ok()
            .flatten()
            == Some(crate::session_activity::State::Active)
    });
    if !active {
        return 3;
    }
    if !same_parent() {
        return 2;
    }
    let opened = rustix::fs::open(
        &target.node,
        rustix::fs::OFlags::RDWR
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::empty(),
    );
    let Ok(fd) = opened else {
        return 5;
    };
    let file = fs::File::from(fd);
    if !file
        .metadata()
        .is_ok_and(|metadata| target.matches(&metadata))
    {
        return 5;
    }
    let Ok(mut device) = evdev::Device::from_fd(file.into()) else {
        return 5;
    };
    if !device
        .supported_ff()
        .is_some_and(|flags| flags.contains(evdev::FFEffectCode::FF_RUMBLE))
        || !device
            .supported_keys()
            .is_some_and(|keys| keys.iter().any(|key| (0x120..0x140).contains(&key.0)))
        || !device
            .supported_absolute_axes()
            .is_some_and(|axes| axes.iter().next().is_some())
    {
        return 4;
    }
    let effect = evdev::FFEffectData {
        direction: 0,
        trigger: Default::default(),
        replay: evdev::FFReplay {
            length: 200,
            delay: 0,
        },
        kind: evdev::FFEffectKind::Rumble {
            strong_magnitude: 0x3000,
            weak_magnitude: 0x3000,
        },
    };
    let Ok(mut effect) = device.upload_ff_effect(effect) else {
        return 5;
    };
    // An upload may outlive a lock; activity checks cannot depend on GUI delivery.
    let states = crate::session_activity::stream();
    iced::futures::pin_mut!(states);
    let active = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(2), states.next())
            .await
            .ok()
            .flatten()
            == Some(crate::session_activity::State::Active)
    });
    if !active {
        return 3;
    }
    if !same_parent() || effect.play(1).is_err() {
        return 5;
    }
    let completed = runtime.block_on(async {
        tokio::select! {
            biased;
            _ = states.next() => false,
            () = tokio::time::sleep(Duration::from_millis(200)) => true,
        }
    });
    if effect.stop().is_err() {
        return 5;
    }
    drop(effect);
    drop(device);
    if !completed {
        return 3;
    }
    println!("ok");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    // The driver lease is process-wide: tests that take it run one at a time.
    static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[tokio::test]
    async fn controller_vibration_behaviour() {
        // Only exact selected event nodes are accepted.
        {
            assert!(event_node(Path::new("/dev/input/event12")));
            for path in [
                "/dev/input/event",
                "/dev/input/event../event1",
                "/tmp/event1",
                "/dev/input/by-id/controller",
                "/dev/input/event123456",
                "/dev/input/event-1",
            ] {
                assert!(!event_node(Path::new(path)), "{path}");
            }
            assert!(Target::capture(Path::new("/dev/null")).is_err());
        }

        // Deferred polling cannot admit a request refused at creation.
        {
            let _serial = SERIAL.lock().await;
            use iced::futures::StreamExt;
            let target = Target {
                node: "/dev/input/event1".into(),
                device: 0,
                inode: 0,
                rdev: 0,
            };
            let first = run_with(Err(std::io::Error::other("Not spawned")), target.clone());
            assert!(has_pending_job());
            let refused = run_with(Err(std::io::Error::other("Not spawned")), target);
            drop(first);
            assert!(!has_pending_job());
            iced::futures::pin_mut!(refused);
            assert!(
                matches!(refused.next().await, Some(Event::Finished(Err(error))) if error == STOPPING)
            );
            assert!(!has_pending_job());
        }
        // Cancelling a driver job kills its owned child.
        {
            let _serial = SERIAL.lock().await;
            use std::os::unix::fs::PermissionsExt;
            let temp = tempfile::tempdir().unwrap();
            let script = temp.path().join("worker");
            let pid_file = temp.path().join("worker.pid");
            fs::write(
                &script,
                "#!/bin/sh\nprintf '%s' \"$$\" > \"$0.pid\"\nexec /bin/sleep 20\n",
            )
            .unwrap();
            fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
            let target = Target {
                node: "/dev/input/event1".into(),
                device: 0,
                inode: 0,
                rdev: 0,
            };
            let second_script = script.clone();
            let second_target = target.clone();
            let task = tokio::spawn(async move {
                use iced::futures::StreamExt;
                let events = run_with(Ok(script), target);
                iced::futures::pin_mut!(events);
                while events.next().await.is_some() {}
            });
            let pid: u32 = tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if let Ok(s) = fs::read_to_string(&pid_file)
                        && let Ok(pid) = s.parse()
                    {
                        break pid;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            use iced::futures::StreamExt;
            let second = run_with(Ok(second_script), second_target);
            iced::futures::pin_mut!(second);
            let denied = tokio::time::timeout(Duration::from_secs(1), second.next())
                .await
                .unwrap();
            assert!(
                matches!(denied, Some(Event::Finished(Err(error))) if error.contains("previous vibration"))
            );
            assert_eq!(fs::read_to_string(&pid_file).unwrap(), pid.to_string());
            let before = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
            let start = before
                .rsplit(')')
                .next()
                .unwrap()
                .split_whitespace()
                .nth(19)
                .unwrap()
                .to_owned();
            task.abort();
            let _ = task.await;
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    let same = fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| {
                        s.rsplit(')').next().unwrap().split_whitespace().nth(19)
                            == Some(start.as_str())
                    });
                    if !same {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
    }
}
