use super::model::Pane;
use super::*;
use crate::Shell;
use enzui_shell::{compositor::ControlGeometry, notifications::model::Key};
use iced::window;

fn shell() -> Shell {
    let compositor = enzui_shell::compositor::Compositor::new(
        std::path::PathBuf::from("/usr/bin/true"),
        "fixture".into(),
    )
    .unwrap();
    Shell::new(compositor, std::path::PathBuf::from("/")).0
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

    let key = Key {
        epoch: 1,
        id: 1,
        revision: 1,
    };
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
