use super::model::{Pane, Reply};
use super::*;
use crate::Shell;
use enzui_shell::{
    compositor::ControlGeometry,
    notifications::{
        model::Key,
        runtime::{self, Update},
        server::{self, Command},
    },
};
use iced::window;
use std::sync::{Arc, atomic::AtomicBool};

fn shell() -> Shell {
    let compositor = enzui_shell::compositor::Compositor::new(
        std::path::PathBuf::from("/usr/bin/true"),
        "fixture".into(),
    )
    .unwrap();
    Shell::new(compositor, std::path::PathBuf::from("/")).0
}
fn key() -> Key {
    Key {
        epoch: 1,
        id: 1,
        revision: 1,
    }
}
fn operation(key: Key, result: Result<(), String>) -> Message {
    Message::Runtime(runtime::Event {
        run: 0,
        update: Update::Operation(server::Event::Action { key, result }),
    })
}
fn toast_shell() -> (Shell, tokio::sync::mpsc::Receiver<Command>) {
    let mut shell = shell();
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    shell.notification.channel = Some(sender);
    shell.notification.authority = Some(Arc::new(AtomicBool::new(true)));
    shell.notification.epoch = Some(1);
    shell.notification.toast = Some(Pane {
        window: window::Id::unique(),
        geometry: ControlGeometry {
            width: 416.0,
            height: 200.0,
            output: "fixture".into(),
            attachment: Default::default(),
        },
        visible: true,
        started: true,
    });
    shell.notification.toast_count = 1;
    (shell, receiver)
}
#[test]
fn notification_panel_behaviour() {
    // Delayed action tokens cannot dispatch after close retry or navigation.
    {
        // Closing the center has no message of its own: it is `None` here.
        for change in [
            None,
            Some(Message::Retry),
            Some(Message::History(true)),
            Some(Message::Page(1)),
        ] {
            let (mut shell, mut receiver) = toast_shell();
            let id = shell.notification.toast.as_ref().unwrap().window;
            let ticket = shell
                .notification
                .actions
                .begin(0, id, key(), "default".into())
                .unwrap();
            assert!(shell.notification.action_pending(&key()));
            let _ = match change {
                Some(change) => shell.notification_message(change),
                None => shell.close_notifications(),
            };
            assert!(!shell.notification.actions.waiting(&key()));
            let _ = shell.notification_invoke_with(
                ticket,
                super::model::Token(Some("stale fixture".into())),
            );
            assert!(receiver.try_recv().is_err());
            assert!(shell.notification.pending.is_empty());
        }
    }
    // Successful reply clears once and late completion cannot clear a new draft.
    {
        let mut shell = shell();
        let key = key();
        shell.notification.reply = Some(Reply {
            key,
            text: "first fixture".into(),
        });
        shell.notification.pending.insert(key, true);
        let _ = shell.notification_message(operation(key, Ok(())));
        assert!(shell.notification.reply.is_none());
        shell.notification.reply = Some(Reply {
            key,
            text: "new fixture".into(),
        });
        let _ = shell.notification_message(operation(key, Ok(())));
        assert_eq!(
            shell.notification.reply.as_ref().unwrap().text,
            "new fixture"
        );
    }
    // Replacing a notification that is on screen updates its toast in place;
    // it does not leave and arrive again (a dictation level changes several
    // times a second).
    //
    // Sabotage: compare whole keys (with the revision) in
    // `notification_track_leaving`.
    {
        use enzui_shell::notifications::{
            model::{Model, Policy},
            origin::Origin,
            payload::Draft,
            server::Snapshot,
        };
        let (mut shell, _commands) = toast_shell();
        let now = std::time::Instant::now();
        let mut model = Model::new(1, Policy::default()).unwrap();
        let origin = Origin::fixture("Dictation", 7);
        let draft = |summary: &str| {
            Draft::parse((
                "Dictation".into(),
                0,
                String::new(),
                summary.into(),
                String::new(),
                vec![],
                Default::default(),
                -1,
            ))
            .unwrap()
        };
        let first = model
            .post(origin.clone(), draft("Listening"), now, 1, false)
            .unwrap();
        let mut snapshot = Snapshot::empty(1, Policy::default());
        snapshot.active = vec![first.notification.clone().unwrap()];
        shell.notification.snapshot = Some(Arc::new(snapshot.clone()));
        shell.notification.toast_shown = shell.notification.toast_display();
        assert_eq!(shell.notification.toast_shown.len(), 1, "positive control");
        shell
            .notification
            .revealed
            .insert((1, first.id), (now, false));
        let mut replacement = draft("Listening, louder");
        replacement.replaces = first.id;
        let second = model.post(origin, replacement, now, 2, false).unwrap();
        assert_eq!(second.id, first.id);
        snapshot.active = vec![second.notification.unwrap()];
        shell.notification.snapshot = Some(Arc::new(snapshot));
        assert!(!shell.notification_track_leaving(), "nothing leaves");
        assert!(shell.notification.toast_leaving.is_empty());
        let shown = shell.notification.toast_display();
        assert_eq!(shown.len(), 1, "one card, not the old one beside the new");
        assert_eq!(shown[0].draft.summary, "Listening, louder");
    }
}

#[test]
fn activity_preview_handoff_keeps_the_panel_until_the_pointer_leaves_both_surfaces() {
    use super::model::Content;

    let mut shell = shell();
    shell.desired_settings.activity_status = true;
    let snapshot = enzui_shell::activity::parse(
        br#"{"op":"snapshot","version":1,"generation":"hover","revision":1,"jobs":[{"id":"1:copy","app_id":"org.example.Files","peer_pid":10,"work":{"job":"copy","label":"Copying files","state":"running","visibility":"always"},"outcome":null,"age_ms":0}]}"#,
    )
    .unwrap();
    let _ = shell.activity_message(crate::shell_activity::ActivityMessage::Service(
        enzui_shell::activity::Event::Snapshot(Box::new(snapshot)),
    ));
    let _ = shell.notification_message(Message::ActivityTriggerHover(true));
    let first_dwell = shell.notification.activity.revision;
    let _ = shell.notification_message(Message::ActivityTriggerHover(false));
    let _ = shell.notification_message(Message::ActivityHoverElapsed(first_dwell));
    assert!(!shell.notification.center_opening);

    let _ = shell.notification_message(Message::ActivityTriggerHover(true));
    let _ = shell.notification_message(Message::ActivityHoverElapsed(
        shell.notification.activity.revision,
    ));
    assert_eq!(shell.notification.content, Content::Activity);
    assert!(shell.notification.center_opening && shell.notification.activity.passive);
    assert!(!shell.notification.blocks_toasts());

    let window = window::Id::unique();
    shell.notification.center_opening = false;
    shell.notification.center = Some(Pane {
        window,
        geometry: ControlGeometry {
            width: 350.0,
            height: 150.0,
            output: "fixture".into(),
            attachment: Default::default(),
        },
        visible: true,
        started: true,
    });

    let _ = shell.notification_message(Message::ActivityTriggerHover(false));
    let gap_dismiss = shell.notification.activity.revision;
    shell.notification.activity.trigger_bounds = Some(iced::Rectangle::new(
        iced::Point::ORIGIN,
        iced::Size::new(90.0, 32.0),
    ));
    let _ = shell.notification_message(Message::ActivityHoverElapsed(gap_dismiss));
    let _ = shell.notification_message(Message::ActivityPointerChecked(gap_dismiss, true));
    assert_eq!(shell.activity_panel_window(), Some(window));

    let _ = shell.notification_message(Message::PopupHover(window, true));
    let _ = shell.notification_message(Message::ActivityPointerChecked(gap_dismiss, false));
    assert_eq!(shell.activity_panel_window(), Some(window));

    let key = key();
    shell
        .notification
        .actions
        .begin(0, window::Id::unique(), key, "default".into())
        .unwrap();
    let _ = shell.notification_message(Message::PopupHover(window, false));
    let dismiss = shell.notification.activity.revision;
    let _ = shell.notification_message(Message::ActivityHoverElapsed(dismiss));
    let _ = shell.notification_message(Message::ActivityPointerChecked(dismiss, false));
    assert!(shell.activity_panel_window().is_none());
    assert!(shell.notification.actions.waiting(&key));
}

#[test]
fn rejected_activity_focus_retries_only_while_the_clicked_panel_is_current() {
    let mut shell = shell();
    shell.notification.content = super::model::Content::Activity;
    shell.notification.center = Some(Pane {
        window: window::Id::unique(),
        geometry: ControlGeometry {
            width: 350.0,
            height: 150.0,
            output: "fixture".into(),
            attachment: Default::default(),
        },
        visible: false,
        started: false,
    });
    let serial = shell.notification.center_serial;
    let rejected = || Message::CenterFocused(serial, 1, Err("not mapped".into()));

    assert_eq!(shell.notification_message(rejected()).units(), 1);
    assert!(shell.notification.error.is_none());
    assert_eq!(
        shell
            .notification_message(Message::RetryActivityFocus(serial, 0))
            .units(),
        1
    );

    shell.notification.activity.passive = true;
    assert_eq!(
        shell
            .notification_message(Message::RetryActivityFocus(serial, 0))
            .units(),
        0
    );

    let _ = shell.close_notifications();
    assert_eq!(
        shell
            .notification_message(Message::RetryActivityFocus(serial, 0))
            .units(),
        0
    );
    assert!(shell.notification.error.is_none());
}
