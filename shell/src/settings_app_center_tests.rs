use super::*;
use enzui_agent::center::{InvocationEvent, test_invocation};
use enzui_shell::settings::{BarEdge, Change, Store};
use enzui_shell::system_settings::Clock;

fn app() -> (SettingsApp, Store, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::at(directory.path().join("settings.json")).unwrap();
    let (mut app, _) = SettingsApp::new(store.clone());
    app.agent = Agent::offline();
    let generation = app.generation;
    let _ = app.update(Message::Loaded(generation, store.load()));
    (app, store, directory)
}

fn call(app: &mut SettingsApp, function: &str, arguments: Value) -> Vec<InvocationEvent> {
    let invocation = test_invocation(function, arguments);
    let _ = app.update(Message::Center(ConnectorEvent::Invoke(Box::new(
        invocation,
    ))));
    app.agent.sent()
}

fn terminal(events: &[InvocationEvent]) -> Option<Result<Value, (String, String)>> {
    events.iter().find_map(|event| match event {
        InvocationEvent::Completed { result, .. } => Some(Ok(result.clone())),
        InvocationEvent::Failed { code, message, .. } => Some(Err((code.clone(), message.clone()))),
        _ => None,
    })
}

#[test]
fn settings_app_center_behaviour() {
    // A change is answered with its saved value and shown at once.
    {
        let (mut app, store, _dir) = app();
        let events = call(
            &mut app,
            "settings.set",
            json!({"key": "shell.bar_position", "value": "bottom"}),
        );
        assert!(
            terminal(&events).is_none(),
            "answered before it was saved: {events:?}"
        );
        assert!(app.busy);
        assert_eq!(
            app.settings.bar,
            BarEdge::Bottom,
            "the window shows the change while it saves"
        );
        let saved = store.update(Change::Bar(BarEdge::Bottom));
        let _ = app.update(Message::Saved(saved));
        let result = terminal(&app.agent.sent()).unwrap().unwrap();
        assert_eq!(
            result,
            json!({"key": "shell.bar_position", "value": "bottom"})
        );
        assert_eq!(store.load().unwrap().bar, BarEdge::Bottom);
        // A windowless worker answers a read with what is on disk now: the shell
        // moved the bar back meanwhile. Sabotage: drop the reload in `serve`.
        store.update(Change::Bar(BarEdge::Top)).unwrap();
        let events = call(
            &mut app,
            "settings.get",
            json!({"key": "shell.bar_position"}),
        );
        let read = terminal(&events).unwrap().unwrap();
        assert_eq!(
            read["value"],
            json!("top"),
            "answered from a stale snapshot: {read}"
        );
    }
    // Invalid unknown and misrouted changes write nothing.
    {
        let (mut app, _store, _dir) = app();
        for (function, arguments, expected) in [
            (
                "settings.set",
                json!({"key": "shell.bar_height", "value": 500}),
                "invalid_value",
            ),
            (
                "settings.set",
                json!({"key": "shell.bar_position", "value": 3}),
                "invalid_value",
            ),
            (
                "settings.set",
                json!({"key": "agents.permissions", "value": true}),
                "unknown_setting",
            ),
            (
                "settings.set",
                json!({"key": "system.time_zone", "value": "UTC"}),
                "wrong_function",
            ),
            (
                "settings.set",
                json!({"key": "shell.bar_position", "value": "top", "grant": "always"}),
                "invalid_arguments",
            ),
            (
                "settings.decide",
                json!({"request_id": "x"}),
                "unknown_function",
            ),
        ] {
            let events = call(&mut app, function, arguments.clone());
            let failure =
                terminal(&events).unwrap_or_else(|| panic!("{arguments} was not answered"));
            assert_eq!(failure.unwrap_err().0, expected, "{arguments}");
            assert!(!app.busy, "{arguments} was written");
        }
    }
    // An ended call withdraws its prompt and a late approval changes nothing.
    {
        let (mut app, _store, _dir) = app();
        let id = ask_for_utc(&mut app);
        let _ = app.update(Message::Center(ConnectorEvent::Cancel {
            invocation_id: id,
            reason: "cancelled by caller".into(),
        }));
        assert!(
            app.agent
                .approving
                .as_ref()
                .is_some_and(|approving| approving.withdraw.is_none()),
            "the prompt is withdrawn"
        );
        // polkit answered before the withdrawal reached it.
        let _ = app.update(Message::AgentApproved(
            1,
            Ok(enzui_shell::authorization::Approval::Approved),
            None,
        ));
        assert!(
            app.agent.approving.is_none(),
            "nothing is committed for an ended call"
        );
        assert!(terminal(&app.agent.sent()).is_none());
    }
}

fn ask_for_utc(app: &mut SettingsApp) -> enzui_agent::center::Uuid {
    app.live_system_backend();
    let invocation = test_invocation(
        "settings.set_system",
        json!({"key": "system.time_zone", "value": "UTC"}),
    );
    let id = invocation.invocation_id.clone();
    let _ = app.update(Message::Center(ConnectorEvent::Invoke(Box::new(
        invocation,
    ))));
    let clock = Clock {
        zones: vec!["UTC".into(), "Europe/Dublin".into()],
        timezone: Some("Europe/Dublin".into()),
        ..Clock::default()
    };
    let _ = app.update(Message::AgentClock(Box::new(Reading::DateTime(clock))));
    assert!(
        app.agent
            .approving
            .as_ref()
            .is_some_and(|approving| approving.withdraw.is_some()),
        "an administrator is asked first"
    );
    assert!(terminal(&app.agent.sent()).is_none());
    id
}
