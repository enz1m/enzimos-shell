use super::*;

#[test]
fn dev_tools_behaviour() {
    // No monitor work exists before a window opens or after it closes.
    {
        let (mut app, _) = App::new();
        let _ = app.update(Message::Tick);
        assert!(app.refresh.is_none());
        assert!(app.client.is_none());
        assert!(app.snapshot.is_none());
        let _ = app.update(Message::Opened(window::Id::unique()));
        assert!(app.refresh.is_some());
        let _ = app.update(Message::Close);
        assert!(app.refresh.is_none());
        assert!(app.window.is_none());
        let _ = app.update(Message::Tick);
        assert!(app.refresh.is_none());
    }
    // Late explicit details and lookup are rejected after view query or close.
    {
        let (mut app, _) = App::new();
        app.window = Some(window::Id::unique());
        app.port_query = "8080".into();
        app.read_epoch = 4;
        let _ = app.update(Message::PortQuery("9090".into()));
        let _ = app.update(Message::LookedUp(4, 8080, Err("old lookup".into())));
        let _ = app.update(Message::Inspected(
            4,
            ProcessId {
                pid: 42,
                start_time: 10,
            },
            Err("old inspect".into()),
        ));
        assert!(app.status.is_none());
        assert!(app.lookup.is_none());
        assert!(app.detail.is_none());
        let current = app.read_epoch;
        let _ = app.update(Message::Close);
        let _ = app.update(Message::Inspected(
            current,
            ProcessId {
                pid: 42,
                start_time: 10,
            },
            Err("closed inspect".into()),
        ));
        assert!(app.status.is_none());
        assert!(!app.explicit_pending);
        assert!(app.explicit.is_none());
    }
}
