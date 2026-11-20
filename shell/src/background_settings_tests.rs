use super::*;
use crate::settings_app::Store;

fn pending(operation: u64) -> Pending {
    let app = serde_json::from_value(serde_json::json!({"schema":1,"app_id":"org.enzui.BackgroundQA","title":"Background QA","purpose":"Public fixture",
        "running":true,"unit_observation_known":true,"visibility":"windowless","window_count":0,"minimized_window_count":0,
        "work_observation_known":false,"work":[],"activation_policy":{"user":true,"autostart":false},
        "autostart_supported":true,"autostart_state":"disabled","graceful_stop_supported":false,"force_stop_supported":true,"identity_verified":true})).unwrap();
    let (focus, _) = ModalFocusLifecycle::open_at::<Parent>(
        &format!("background-test-{operation}"),
        Some("background-test-invoker".into()),
        "background-review-cancel".into(),
    );
    Pending {
        operation,
        action: Action::Stop {
            app,
            mode: StopMode::Force,
        },
        result: None,
        prepared: None,
        focus,
    }
}

#[test]
fn an_old_confirmation_cancel_or_exit_cannot_touch_a_reopened_review() {
    let directory = tempfile::tempdir().unwrap();
    let (mut app, _) = SettingsApp::new(Store::at(directory.path().join("settings.json")).unwrap());
    app.page = Page::Apps;
    app.background.operation = 10;
    app.background.review.present(pending(10));
    let old_key = app.background.review.key().unwrap();
    let _ = app.background_message(Message::Cancel(10));
    assert!(!app.background.review.is_open());
    app.background.operation = 11;
    app.background.review.present(pending(11));
    let _ = app.background_message(Message::Settled(old_key));
    let _ = app.background_message(Message::Confirm(10));
    let _ = app.background_message(Message::Cancel(10));
    let _ = app.background_message(Message::Host(
        10,
        ModalHostEvent::Dismissed(enzui_core::modal::DismissCause::CancelAction),
    ));
    assert!(app.background.review.is_open());
    assert_eq!(app.background.review.value().unwrap().operation, 11);
    assert!(!app.background.committing);
    let _ = app.background_message(Message::Cancel(11));
    assert!(!app.background.review.is_open());
}
