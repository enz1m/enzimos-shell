use super::*;

/// Requests to greetd, its replies, and the socket that turns readable when a
/// reply arrives, so a login screen waiting for nothing sleeps.
pub(super) type GreeterLink = (
    mpsc::Sender<greetd::Request>,
    mpsc::Receiver<greetd::Event>,
    Option<std::os::unix::net::UnixStream>,
);

pub(super) fn start_greeter(ui: &mut ui::Ui) -> GreeterLink {
    let users = accounts::users();
    let sessions = accounts::sessions();
    let (last_user, last_session) = accounts::last();
    let user = last_user
        .and_then(|n| users.iter().position(|u| u.name == n))
        .unwrap_or(0);
    let session = last_session
        .and_then(|id| sessions.iter().position(|s| s.id == id))
        .unwrap_or(0);
    let (requests, receiver) = mpsc::channel();
    let (sender, replies) = mpsc::channel();
    std::thread::spawn(move || greetd::run(receiver, sender));
    let (events, wake) = ringing(replies);
    let empty = users.is_empty();
    let preview = std::env::var("ENZUI_SHELL_PLATFORM_PREVIEW").as_deref() == Ok("1");
    ui.greeter = Some(ui::Greeter {
        users,
        user,
        sessions,
        session,
        prompt: None,
        notice: None,
        power: !preview,
        existing: None,
    });
    if empty {
        ui.error = Some((
            tr("shell-no-user-account-can-sign-in-here").into(),
            Instant::now(),
        ));
    } else {
        begin(ui, &requests);
    }
    (requests, events, wake)
}

fn begin(ui: &mut ui::Ui, requests: &mpsc::Sender<greetd::Request>) {
    let Some(greeter) = ui.greeter.as_mut() else {
        return;
    };
    greeter.prompt = None;
    greeter.notice = None;
    greeter.existing = None;
    let Some(user) = greeter.users.get(greeter.user) else {
        return;
    };
    // Already signed in on this seat (fast user switching): its own lock
    // screen asks for the password, so no second desktop is started.
    if switch::available()
        && let Some(id) = switch::existing_session(&user.name)
    {
        greeter.existing = Some(id);
        greeter.notice =
            Some(tr("shell-already-signed-in-its-lock-screen-asks-for-the-password").into());
        let _ = requests.send(greetd::Request::Cancel);
        return;
    }
    let _ = requests.send(greetd::Request::Begin(user.name.clone()));
}

pub(super) fn greeter_action(
    ui: &mut ui::Ui,
    action: ui::Message,
    requests: &mpsc::Sender<greetd::Request>,
) {
    if ui.phase != ui::Phase::Idle {
        return;
    }
    let Some(greeter) = ui.greeter.as_mut() else {
        return;
    };
    let users = greeter.users.len().max(1);
    match action {
        ui::Message::PreviousUser | ui::Message::NextUser if users > 1 => {
            let step = if matches!(action, ui::Message::NextUser) {
                1
            } else {
                users - 1
            };
            greeter.user = (greeter.user + step) % users;
            ui.password.clear();
            ui.error = None;
            ui.last_input = Some(Instant::now());
            begin(ui, requests);
        }
        ui::Message::NextSession if !greeter.sessions.is_empty() => {
            greeter.session = (greeter.session + 1) % greeter.sessions.len();
            ui.last_input = Some(Instant::now());
        }
        ui::Message::PowerOff | ui::Message::Restart | ui::Message::Sleep if greeter.power => {
            let verb = match action {
                ui::Message::PowerOff => "poweroff",
                ui::Message::Sleep => "suspend",
                _ => "reboot",
            };
            let _ = std::process::Command::new("systemctl")
                .arg(verb)
                .stdin(std::process::Stdio::null())
                .spawn();
        }
        _ => {}
    }
}

pub(super) fn launch(ui: &mut ui::Ui, requests: &mpsc::Sender<greetd::Request>) {
    let Some(session) = ui
        .greeter
        .as_ref()
        .and_then(|g| g.sessions.get(g.session))
        .cloned()
    else {
        let _ = greetd_failure(
            ui,
            tr("shell-no-desktop-session-is-installed").into(),
            requests,
        );
        return;
    };
    let command = match session.command() {
        Ok(command) => command,
        Err(reason) => {
            let _ = greetd_failure(
                ui,
                tr!("shell-can-t-be-started", "arg0" => session.name, "arg1" => reason),
                requests,
            );
            return;
        }
    };
    let desktop = if session.desktop_names.is_empty() {
        session.id.clone()
    } else {
        session.desktop_names.clone()
    };
    let _ = requests.send(greetd::Request::Launch {
        command,
        env: vec![
            "XDG_SESSION_TYPE=wayland".into(),
            format!("XDG_SESSION_DESKTOP={}", session.id),
            format!("XDG_CURRENT_DESKTOP={desktop}"),
        ],
    });
}

fn greetd_failure(
    ui: &mut ui::Ui,
    message: String,
    requests: &mpsc::Sender<greetd::Request>,
) -> bool {
    if matches!(ui.phase, ui::Phase::Leaving(_)) {
        ui.started = Instant::now();
    }
    ui.phase = ui::Phase::Idle;
    ui.password.clear();
    ui.error = Some((message, Instant::now()));
    ui.last_input = Some(Instant::now());
    begin(ui, requests);
    false
}

pub(super) fn greeter_event(
    ui: &mut ui::Ui,
    event: greetd::Event,
    requests: &mpsc::Sender<greetd::Request>,
    launching: &mut bool,
) -> bool {
    match event {
        greetd::Event::Prompt { text, secret } => {
            if ui.phase == ui::Phase::Checking {
                // PAM asks again (a new password, a second factor).
                ui.phase = ui::Phase::Idle;
                ui.last_input = Some(Instant::now());
            }
            if let Some(greeter) = ui.greeter.as_mut() {
                greeter.prompt = Some((text, secret));
            }
        }
        greetd::Event::Notice(text) => {
            if let Some(greeter) = ui.greeter.as_mut() {
                greeter.notice = Some(text);
            }
        }
        greetd::Event::Failed(_) => {
            ui.phase = ui::Phase::Idle;
            ui.password.clear();
            ui.error = Some((
                tr("shell-incorrect-password-try-again").into(),
                Instant::now(),
            ));
            if let Some(greeter) = ui.greeter.as_mut() {
                greeter.prompt = None;
            }
        }
        greetd::Event::Authenticated => ui.phase = ui::Phase::Leaving(Instant::now()),
        greetd::Event::Launched => {
            if let Some(greeter) = ui.greeter.as_ref()
                && let (Some(user), Some(session)) = (
                    greeter.users.get(greeter.user),
                    greeter.sessions.get(greeter.session),
                )
            {
                accounts::remember(&user.name, &session.id);
            }
            return true;
        }
        greetd::Event::Error(text) => {
            *launching = false;
            return greetd_failure(ui, text, requests);
        }
    }
    false
}

/// Forwards `replies` to the returned receiver and makes the returned socket
/// readable after each one, so the loop can sleep in `poll` until greetd
/// answers. Without a socket the loop has to poll the receiver.
fn ringing<T: Send + 'static>(
    replies: mpsc::Receiver<T>,
) -> (mpsc::Receiver<T>, Option<std::os::unix::net::UnixStream>) {
    let (forward, events) = mpsc::channel();
    let Some((wake, ring)) = std::os::unix::net::UnixStream::pair()
        .ok()
        .filter(|(wake, ring)| {
            wake.set_nonblocking(true).is_ok() && ring.set_nonblocking(true).is_ok()
        })
    else {
        std::thread::spawn(move || {
            for reply in replies {
                if forward.send(reply).is_err() {
                    break;
                }
            }
        });
        return (events, None);
    };
    std::thread::spawn(move || {
        for reply in replies {
            if forward.send(reply).is_err() {
                break;
            }
            // A full socket already holds a wake the loop has not read.
            let _ = (&ring).write(&[1]);
        }
    });
    (events, Some(wake))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    /// A greetd reply makes the wake socket readable, after the reply can be
    /// received, so the loop that wakes finds it.
    ///
    /// Sabotage: drop the write to `ring`. The poll then times out.
    #[test]
    fn a_greetd_reply_wakes_the_loop() {
        let (replies_tx, replies) = mpsc::channel();
        let (events, wake) = ringing(replies);
        let wake = wake.expect("a socket pair");
        let mut fds = [rustix::event::PollFd::new(
            &wake,
            rustix::event::PollFlags::IN,
        )];
        let quick = rustix::event::Timespec {
            tv_sec: 0,
            tv_nsec: 50_000_000,
        };
        assert_eq!(rustix::event::poll(&mut fds, Some(&quick)).ok(), Some(0));

        replies_tx.send(7_u8).expect("forwarder alive");
        let long = rustix::event::Timespec {
            tv_sec: 5,
            tv_nsec: 0,
        };
        assert_eq!(rustix::event::poll(&mut fds, Some(&long)).ok(), Some(1));
        assert_eq!(events.try_recv().ok(), Some(7));
        let mut drained = [0_u8; 8];
        assert_eq!((&wake).read(&mut drained).ok(), Some(1));
    }
}
