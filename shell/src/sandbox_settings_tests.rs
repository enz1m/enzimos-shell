use super::*;

fn fixture() -> (tempfile::TempDir, SettingsApp) {
    let directory = tempfile::tempdir().unwrap();
    let (app, _) = SettingsApp::new(
        enzui_shell::settings::Store::at(directory.path().join("settings.json")).unwrap(),
    );
    (directory, app)
}
fn snapshot() -> Snapshot {
    Snapshot {
        app_id: "org.example.App".into(),
        scope: "user".into(),
        review_token: "a".into(),
        entries: vec![],
        unknown: vec![],
        limitations: vec![],
        writable: true,
        layers: vec![],
    }
}
fn grant() -> Grant {
    Grant {
        table: "devices".into(),
        resource: "camera".into(),
        values: vec!["yes".into()],
    }
}
fn prepare(app: &mut SettingsApp) {
    app.store_system_reading(enzui_shell::system_settings::Reading::Apps(
        enzui_shell::system_settings::Apps {
            permissions: enzui_shell::app_permissions::Permissions {
                apps: Some(vec![enzui_shell::app_permissions::App {
                    id: "org.example.App".into(),
                    name: "QA".into(),
                    grants: vec![],
                    sandbox: vec![],
                    icon: None,
                }]),
                ..Default::default()
            },
            ..Default::default()
        },
    ));
    app.sandbox.open = Some("org.example.App".into());
    app.sandbox.generation = 2;
    app.sandbox.snapshot = Some(Ok(snapshot()));
    app.sandbox.portals = Some(Ok(Portals {
        app_id: "org.example.App".into(),
        review_token: "p".into(),
        grants: vec![grant()],
        absent: vec![],
    }));
}
#[test]
fn sandbox_settings_behaviour() {
    // Retired review controls cannot mutate the reopened app.
    {
        let (_directory, mut app) = fixture();
        prepare(&mut app);
        let _ = app.sandbox_message(Message::Edit(
            1,
            "a".into(),
            Edit {
                category: Category::Shared,
                value: "network".into(),
                allow: true,
            },
        ));
        let _ = app.sandbox_message(Message::Reset(1, "a".into()));
        let _ = app.sandbox_message(Message::Answer(1, "p".into(), grant(), Some(false)));
        let _ = app.sandbox_message(Message::Edit(
            2,
            "old-review".into(),
            Edit {
                category: Category::Shared,
                value: "network".into(),
                allow: true,
            },
        ));
        let _ = app.sandbox_message(Message::Answer(
            2,
            "old-review".into(),
            grant(),
            Some(false),
        ));
        assert!(app.sandbox.pending.is_none());
        assert!(!app.sandbox.busy);
        let _ = app.sandbox_message(Message::Applied(
            1,
            1,
            "org.example.App".into(),
            Err("old".into()),
        ));
        assert!(app.sandbox.error.is_none());
    }
    // Failed writes keep readback but disable mutation until refresh.
    {
        let (_directory, mut app) = fixture();
        prepare(&mut app);
        app.sandbox.pending = Some(4);
        app.sandbox.busy = true;
        let _ = app.sandbox_message(Message::Applied(
            2,
            4,
            "org.example.App".into(),
            Err("unknown outcome".into()),
        ));
        assert_eq!(
            app.sandbox
                .snapshot
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .review_token,
            "a"
        );
        assert!(app.sandbox.error.is_some());
        assert!(!app.sandbox.busy);
        let _ = app.sandbox_message(Message::Reset(2, "a".into()));
        assert!(app.sandbox.pending.is_none());
        app.sandbox.error = None;
        app.sandbox.pending = Some(5);
        let _ = app.sandbox_message(Message::Answered(
            2,
            4,
            "org.example.App".into(),
            Ok(Portals {
                app_id: "org.example.App".into(),
                review_token: "old".into(),
                grants: vec![],
                absent: vec![],
            }),
        ));
        assert_eq!(app.sandbox.pending, Some(5));
        assert_eq!(
            app.sandbox
                .portals
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .review_token,
            "p"
        );
    }
}
